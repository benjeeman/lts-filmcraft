//! The planner (spec sections 7.1-7.6): the one pure function behind every Vertical Edit write.
//!
//! [`plan_write`] takes the project (it says which clips show picture, so which edit points are walls), the sequence as it
//! is now (the engine raises the set's id counters to the session's high-water marks in a working copy first, plan 63
//! C-11), one [`Write`] and the [`Caller`]. It returns the sequence's new vertical data and what changed, or a
//! [`Refusal`] that changes nothing. A dry run is the same call whose result the engine does not apply, so it is identical
//! by construction. A write that changes nothing returns `changed: false` with the set as it was.
//!
//! Frames are sequence frame indices. The engine resolves `time` first (refused `bad_time` off a frame start, see
//! [`frame_of_time`]), `timecode` and part timecodes ([`parse_part_timecode`](super::parts::parse_part_timecode)), and
//! refuses `seconds` (`use_frame`). Every number of a write is the caller's and is treated as hostile.
//!
//! In M2 the engine always plans with [`PERSON`]. The agent-only branches of the spec (`human_only`, the overwrite gate and
//! overwritten ranges, `expect_required`, proposals) are additive code in M5 behind the same signature (plan 63 section 5).

use std::collections::{BTreeMap, BTreeSet};

use filmcraft_time::{FrameRate, Tick, format_timecode_frames};
use serde::Serialize;
use serde_json::json;

use super::annotations::{set_annotations, valid_track_id};
use super::area::{active_rect, clamp_to_area, set_active_area};
use super::canon::{centre_bounds, clamp_window, h_from_zoom, q, quantize_window};
use super::curve::{DEFAULT_CURVE, check_curve, curve_name};
use super::derive::{Facts, Scene, from_label_in, plan_issues_near, raster, touches};
use super::digest::{data_digest, effective_walls_digest_in, parts_digest, plan_digest, range_digest_in, regions_digest, walls_digest};
use super::eval::{centred_largest, limits, window_at};
use super::measure::{Fits, corner_deltas, move_fits, speed_of, whole_move_numbers};
use super::parts::{approve_part, set_parts};
use super::regions::set_regions;
use super::smooth::smooth_through;
use super::split::{insert_split, split_move};
use super::types::{
    Arrival, AttentionMark, Caller, CaptionAnchor, Change, Curve, DeliveryInfo, Filter, FitBackground, Issue, Layout, Level, Limits, MAX_COORDINATE, Origin,
    OutputSize, PartAudio, PartMeta, Position, Raster, Refusal, RegionMode, SmoothOpts, VE_MAX_FRAMES, VerticalCaptions, VerticalPlan, VerticalSet, WallOrigin,
    WallSet, Window, Writer,
};
use super::validate::{MAX_POSITIONS, Violation, plan_only_violations, plan_violations, repair, same_aspect, set_violations, validate};
use super::walls::{apply_measurement, plural, set_walls};
use crate::{Label, MarkerKind, Project, Sequence, validate_frame_size};

/// The caller of every M2 write: a person. Nothing sets `Session.writer` to `Agent` before M5.
pub const PERSON: Caller = Caller { writer: Writer::Human, overwrite_allowed: false };

/// The most ids or frames one list of a write may name (a plan holds at most 10 000 positions).
pub const MAX_LIST: usize = 10_000;

/// The most items one `vertical.replaceRange` may carry.
pub const MAX_REPLACE_ITEMS: usize = 2_000;

/// One write of the Vertical Edit, as the engine parsed it: one variant per write command of plan 63 section 3.2 except
/// `vertical.remap` (lane L1c's). A write carries no `dryRun` (a dry run is the same plan, not applied), no `runId` (M5)
/// and no `merge` (the engine's gesture key, `edit_plan_only`).
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Write {
    Create(CreatePlan),
    Update(UpdatePlan),
    Delete(DeletePlan),
    SetPosition(SetPosition),
    RetimePosition(RetimePosition),
    DeletePositions(DeletePositions),
    ReplaceRange(ReplaceRange),
    SetArrival(SetArrival),
    SmoothThrough(SmoothThrough),
    FitMove(FitMove),
    SwapWindows(SwapWindows),
    SetWalls(SetWalls),
    ApplyMeasurement(ApplyMeasurement),
    SeedShots(SeedShots),
    SetParts(SetParts),
    SetRegions(SetRegions),
    MarkChecked(MarkChecked),
    Repair(Repair),
    SetAnnotations(SetAnnotations),
    SetActiveArea(SetActiveArea),
    ApprovePart(ApprovePart),
    SetReview(SetReview),
    SetLock(SetLock),
    FlagAttention(FlagAttention),
    ClearAttention(ClearAttention),
}

/// The output presets of `vertical.create` (spec section 5.14).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Preset {
    /// 9:16, 1080x1920.
    #[default]
    NineSixteen,
    /// 4:5, 1080x1350.
    FourFive,
    /// 1:1, 1080x1080.
    OneOne,
}

impl Preset {
    /// The preset's name, the default plan name: "9:16", "4:5" or "1:1".
    pub fn name(self) -> &'static str {
        match self {
            Self::NineSixteen => "9:16",
            Self::FourFive => "4:5",
            Self::OneOne => "1:1",
        }
    }

    /// The preset's output size.
    pub fn size(self) -> OutputSize {
        match self {
            Self::NineSixteen => OutputSize { width: 1080, height: 1920 },
            Self::FourFive => OutputSize { width: 1080, height: 1350 },
            Self::OneOne => OutputSize { width: 1080, height: 1080 },
        }
    }
}

/// `vertical.create {sequence, name, preset, width, height, filter}`: a new plan with no positions. Without a size the
/// preset's (9:16 when none); a size needs both sides. The name defaults to the output's reduced ratio ("9:16").
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CreatePlan {
    pub name: Option<String>,
    pub preset: Option<Preset>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub filter: Option<Filter>,
}

/// `vertical.update {plan, name, width, height, filter, startingWindow, captions, fitBackground, clamp, expect}`.
/// `starting_window: Some(None)` clears it (JSON null).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UpdatePlan {
    pub plan: u32,
    pub name: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub filter: Option<Filter>,
    pub starting_window: Option<Option<Window>>,
    pub captions: Option<VerticalCaptions>,
    pub fit_background: Option<FitBackground>,
    pub clamp: bool,
    pub expect: Option<String>,
}

/// `vertical.delete {plan, expect}`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeletePlan {
    pub plan: u32,
    pub expect: Option<String>,
}

/// How a write says the window arrives: `"cut"` or `"move"`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ArrivalKind {
    #[default]
    Cut,
    Move,
}

/// The position keys a write may give (spec section 7.2). `None` is an omitted key: a window component omitted is taken
/// from the window shown at the frame; omitted metadata keeps its value. `Some(None)` (or an empty string or list) is an
/// explicit null, which clears it. A move written without `curve` gets Ramp 20/20 ([`DEFAULT_CURVE`](super::DEFAULT_CURVE))
/// when it is new and keeps its curve when it was a move already.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PositionFields {
    pub cx: Option<f64>,
    pub cy: Option<f64>,
    pub h: Option<f64>,
    /// Percent of the largest window: `h = q(h_max_g * 100 / zoom)`. Never with `h`.
    pub zoom: Option<f64>,
    pub layout: Option<Layout>,
    pub arrival: Option<ArrivalKind>,
    pub move_frames: Option<i64>,
    pub curve: Option<Curve>,
    pub caption_anchor: Option<Option<CaptionAnchor>>,
    pub label: Option<String>,
    pub color: Option<Option<Label>>,
    pub tags: Option<Vec<String>>,
    pub note: Option<String>,
    /// Box-track ids (the engine exposes the key in M5, with box tracks).
    pub subjects: Option<Vec<String>>,
    pub focus: Option<Option<[f64; 2]>>,
    /// The agent's (the engine exposes the key in M5).
    pub confidence: Option<Option<f64>>,
    /// The agent's (the engine exposes the key in M5).
    pub risk: Option<String>,
}

/// `expectRange {fromFrame, toFrame, digest}`: the caller's range digest of `[from_frame, to_frame)`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RangeGuard {
    pub from_frame: i64,
    pub to_frame: i64,
    pub digest: String,
}

/// `vertical.setPosition`: upsert by frame. Inside a move with no position at the frame it makes the exact split of spec
/// section 3.6 first; a given arrival that disagrees with the split is `split_conflict`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SetPosition {
    pub plan: u32,
    pub frame: i64,
    pub fields: PositionFields,
    /// Copy the window and layout of the label's nearest earlier occurrence before the frame's shot (spec section 5.11).
    pub from_label: Option<String>,
    /// Copy the window and layout of this position.
    pub from_position: Option<u32>,
    pub clamp: bool,
    pub expect: Option<String>,
    pub expect_range: Option<RangeGuard>,
}

/// `vertical.retimePosition`: moves a position in time, keeping its id, window and move length.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RetimePosition {
    pub plan: u32,
    pub position: u32,
    pub frame: i64,
    pub expect: Option<String>,
    pub expect_range: Option<RangeGuard>,
}

/// `vertical.deletePositions`: missing ids are listed in [`Planned::missing`], not refused.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeletePositions {
    pub plan: u32,
    pub positions: Vec<u32>,
    pub expect: Option<String>,
    pub expect_range: Option<RangeGuard>,
}

/// One position of `vertical.replaceRange`: its frame and keys (`cx`, `cy` and `h` or `zoom` are required).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReplaceItem {
    pub frame: i64,
    pub fields: PositionFields,
}

/// `vertical.replaceRange`: atomically replaces every position with a frame in `[from_frame, to_frame)`; a matched frame
/// keeps its id. `strict` refuses `range_exceeded` when the changed frames leave the range.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReplaceRange {
    pub plan: u32,
    pub from_frame: i64,
    pub to_frame: i64,
    pub items: Vec<ReplaceItem>,
    pub clamp: bool,
    pub strict: bool,
    pub expect: Option<String>,
    pub expect_range: Option<RangeGuard>,
    /// The `effectiveWallsDigest` the caller read.
    pub expect_walls: Option<String>,
}

/// `vertical.setArrival`: how the window arrives at each of several positions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SetArrival {
    pub plan: u32,
    pub positions: Vec<u32>,
    pub arrival: ArrivalKind,
    pub move_frames: Option<i64>,
    pub curve: Option<Curve>,
    pub expect: Option<String>,
    pub expect_range: Option<RangeGuard>,
}

/// `vertical.smoothThrough` (spec section 3.7). Walls are checked first (`move_crosses_wall`), then the run.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SmoothThrough {
    pub plan: u32,
    pub positions: Vec<u32>,
    pub opts: SmoothOpts,
    pub expect: Option<String>,
    pub expect_range: Option<RangeGuard>,
}

/// `vertical.fitMove` (spec section 3.8): the shortest move into `position` whose measured peak speed (and acceleration,
/// when given) meets the limits, in output px per frame (per frame squared).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FitMove {
    pub plan: u32,
    pub position: u32,
    pub max_speed: f64,
    pub max_accel: Option<f64>,
    pub curve: Option<Curve>,
    pub expect: Option<String>,
    pub expect_range: Option<RangeGuard>,
}

/// `vertical.swapWindows`: exchanges two positions' windows and layouts, keeping their times and arrivals.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SwapWindows {
    pub plan: u32,
    pub positions: Vec<u32>,
    pub expect: Option<String>,
    pub expect_range: Option<RangeGuard>,
}

/// What a wall write does with a new wall inside an existing move (W5): refuse it (`would_split_move`, the API default)
/// or shorten the move to start at the wall (what the UI passes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum OnConflict {
    #[default]
    Refuse,
    Shorten,
}

/// A wall to store at a frame, with a score when the caller's detector gave one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WallAdd {
    pub frame: i64,
    pub score: Option<f64>,
}

/// A stored wall moved from one frame to another (W8).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct WallMove {
    pub from: i64,
    pub to: i64,
}

/// A possible cut at a frame, with its score.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CandidateAt {
    pub frame: i64,
    pub score: f64,
}

/// `candidates {fromFrame, toFrame, set}`: replaces the candidates inside the range.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CandidateSet {
    pub from_frame: i64,
    pub to_frame: i64,
    pub set: Vec<CandidateAt>,
}

/// `fromMarkers {name, color, kind}`: the sequence markers to copy once as `marker` walls (every given key must match).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MarkerFilter {
    pub name: Option<String>,
    pub color: Option<Label>,
    pub kind: Option<MarkerKind>,
}

/// `vertical.setWalls`. Applied in this order: `use_edit_points`, `remove`, `unsuppress`, `suppress`, `add`, `promote`,
/// `from_markers`, `moves`, `candidates`, `analysed`; then W5 for every new effective wall inside a move. `expect` is the
/// `wallsDigest`, `expect_walls` the `effectiveWallsDigest`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SetWalls {
    /// A wall of the caller's origin on each frame. A suppression of the frame is lifted: a detected wall a person deleted
    /// comes back as the caller's (Alt+B on it again).
    pub add: Vec<WallAdd>,
    /// A `detected` wall is suppressed; a `manual`, `agent`, `marker` or `reanchored` one is removed.
    pub remove: Vec<i64>,
    /// W8: a stored wall that is not suppressed moves, never onto a suppressed frame or onto a position's frame while it
    /// carries another position there (`wall_blocked`).
    pub moves: Vec<WallMove>,
    /// A candidate becomes a wall of the caller's origin with the candidate's score; a suppression of its frame is lifted.
    pub promote: Vec<i64>,
    pub suppress: Vec<i64>,
    pub unsuppress: Vec<i64>,
    pub candidates: Option<CandidateSet>,
    pub analysed: Vec<[i64; 2]>,
    pub use_edit_points: Option<bool>,
    pub from_markers: Option<MarkerFilter>,
    pub on_conflict: OnConflict,
    pub expect: Option<String>,
    pub expect_walls: Option<String>,
}

/// A frame a shot-change measurement found, with its frame difference.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MeasuredFrame {
    pub frame: i64,
    pub score: f64,
}

/// A shot-change measurement as the planner needs it (spec section 4.2): the range it analysed, `[from_frame, to_frame)`,
/// its cuts and its possible cuts. The engine keeps the measurement itself and refuses `stale_measurement` before it plans.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Measurement {
    pub from_frame: i64,
    pub to_frame: i64,
    pub cuts: Vec<MeasuredFrame>,
    pub candidates: Vec<MeasuredFrame>,
}

/// `vertical.applyMeasurement`: inside `[from_frame, to_frame)` (default: the measurement's range) the stored `detected`
/// walls and the candidates become the measurement's; every other wall and every suppression stays.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ApplyMeasurement {
    pub measurement: Measurement,
    pub from_frame: Option<i64>,
    pub to_frame: Option<i64>,
    pub on_conflict: OnConflict,
    pub expect_walls: Option<String>,
}

/// `vertical.seedShots` modes (spec section 4.5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SeedMode {
    /// The window already shown on the shot's first frame.
    #[default]
    Carry,
    /// The plan's starting window, or the centred largest one, inside the active area.
    Starting,
}

/// `vertical.seedShots`: an opening cut on every wall (and frame 0) in range that has none; origin `seeded`, unreviewed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SeedShots {
    pub plan: u32,
    pub from_frame: Option<i64>,
    pub to_frame: Option<i64>,
    pub mode: SeedMode,
    pub expect: Option<String>,
}

/// A part to add. An add identical to an existing part (name, range, audio, meta) returns that part's id and changes
/// nothing; the same name with other content is `part_exists {id}`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PartAdd {
    pub name: String,
    pub from_frame: i64,
    pub to_frame: i64,
    pub audio: Option<PartAudio>,
    pub meta: Option<PartMeta>,
}

/// A part to change; `audio: Some(None)` drops its own fades (JSON null).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PartUpdate {
    pub part: u32,
    pub name: Option<String>,
    pub from_frame: Option<i64>,
    pub to_frame: Option<i64>,
    pub audio: Option<Option<PartAudio>>,
    pub meta: Option<PartMeta>,
}

/// `moveEdges {frame, to}`: every part edge on `frame` moves to `to` (shared edges move together).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct EdgeMove {
    pub frame: i64,
    pub to: i64,
}

/// `fromMarkers {kind: "segmentation", color}`: copies, once, the segmentation markers (of that colour) as parts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PartMarkers {
    pub color: Option<Label>,
}

/// `vertical.setParts`. Applied in this order: `remove`, `update`, `add`, `from_markers`, `move_edges`; then W5 for every
/// new boundary inside a move. `expect` is the `partsDigest` ([`parts_digest`](super::digest::parts_digest)).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SetParts {
    pub add: Vec<PartAdd>,
    pub update: Vec<PartUpdate>,
    pub remove: Vec<u32>,
    pub from_markers: Option<PartMarkers>,
    pub move_edges: Vec<EdgeMove>,
    pub on_conflict: OnConflict,
    pub expect: Option<String>,
}

/// A protected region to add. An add identical to an existing region (label, range, rect, mode) returns its id.
#[derive(Clone, Debug, PartialEq)]
pub struct RegionAdd {
    pub label: String,
    pub from_frame: i64,
    pub to_frame: i64,
    pub rect: [f64; 4],
    pub mode: RegionMode,
}

/// A protected region to change.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RegionUpdate {
    pub region: u32,
    pub label: Option<String>,
    pub from_frame: Option<i64>,
    pub to_frame: Option<i64>,
    pub rect: Option<[f64; 4]>,
    pub mode: Option<RegionMode>,
}

/// `vertical.setRegions`. Applied in this order: `remove`, `update`, `add`. `expect` is the `regionsDigest`
/// ([`regions_digest`](super::digest::regions_digest)).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SetRegions {
    pub add: Vec<RegionAdd>,
    pub update: Vec<RegionUpdate>,
    pub remove: Vec<u32>,
    pub expect: Option<String>,
}

/// `vertical.markChecked`: clears the flags of the positions (all of the plan's when `None`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MarkChecked {
    pub plan: u32,
    pub positions: Option<Vec<u32>>,
    pub expect: Option<String>,
    pub expect_range: Option<RangeGuard>,
}

/// `vertical.repair`: the deterministic repair of spec section 2.8; never changes a locked position.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Repair {
    pub plan: u32,
}

/// An annotation to add. An add identical to an existing one (kind, range, label, score) returns its id.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnnotationAdd {
    pub kind: String,
    pub from_frame: i64,
    pub to_frame: i64,
    pub label: String,
    pub score: Option<f64>,
}

/// An annotation to change; `score: Some(None)` clears it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnnotationUpdate {
    pub id: u32,
    pub kind: Option<String>,
    pub from_frame: Option<i64>,
    pub to_frame: Option<i64>,
    pub label: Option<String>,
    pub score: Option<Option<f64>>,
}

/// One annotation of a `replace` set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnnotationItem {
    pub from_frame: i64,
    pub to_frame: i64,
    pub label: String,
    pub score: Option<f64>,
}

/// `replace {kind, fromFrame, toFrame, set}`: every annotation of `kind` lying inside the range is replaced by `set`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnnotationReplace {
    pub kind: String,
    pub from_frame: i64,
    pub to_frame: i64,
    pub set: Vec<AnnotationItem>,
}

/// `vertical.setAnnotations`. Applied in this order: `remove`, `update`, `replace`, `add`. `expect` is the `dataDigest`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SetAnnotations {
    pub add: Vec<AnnotationAdd>,
    pub update: Vec<AnnotationUpdate>,
    pub remove: Vec<u32>,
    pub replace: Option<AnnotationReplace>,
    pub expect: Option<String>,
}

/// One active area to set over `[from_frame, to_frame)`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AreaSet {
    pub from_frame: i64,
    pub to_frame: i64,
    pub rect: [f64; 4],
}

/// `vertical.setActiveArea {set, clear}`: `clear` first, then each `set` over its range (an area set over frames that had
/// another replaces it there). `expect` is the `dataDigest`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SetActiveArea {
    pub set: Vec<AreaSet>,
    pub clear: Vec<[i64; 2]>,
    pub expect: Option<String>,
}

/// `vertical.approvePart {plan, part, note}`; `by` (the editor-name preference) and `when` (the session's clock, Unix
/// seconds) come from the engine.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ApprovePart {
    pub plan: u32,
    pub part: u32,
    pub note: String,
    pub by: String,
    pub when: i64,
}

/// `vertical.setReview {plan, positions, shotFrame, reviewed}`. With `shot_frame` and `reviewed: true` every position of
/// that shot is marked reviewed, a `SAME` opener is added first when the shot has none, and the shot's attention marks are
/// cleared: the one undo step of Alt+R (spec section 5.10).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SetReview {
    pub plan: u32,
    pub positions: Vec<u32>,
    pub shot_frame: Option<i64>,
    pub reviewed: bool,
}

/// `vertical.setLock {plan, positions, locked}`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SetLock {
    pub plan: u32,
    pub positions: Vec<u32>,
    pub locked: bool,
}

/// `vertical.flagAttention {plan, frame, note}`; `by` and `when` come from the engine.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FlagAttention {
    pub plan: u32,
    pub frame: i64,
    pub note: String,
    pub by: String,
    pub when: i64,
}

/// `vertical.clearAttention {plan, frames}`: frames without a mark are not an error.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClearAttention {
    pub plan: u32,
    pub frames: Vec<i64>,
}

/// What a write did to one plan's positions: ids added, changed and removed (spec section 7.5), and the frames whose
/// window or layout differ, merged and half-open.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanDelta {
    pub plan: u32,
    pub added: Vec<u32>,
    pub updated: Vec<u32>,
    pub removed: Vec<u32>,
    pub changed_frames: Vec<[i64; 2]>,
}

/// A window a write with `clamp` moved inward: by how much each value moved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Clamped {
    pub position: u32,
    pub dx: f64,
    pub dy: f64,
    pub dh: f64,
}

/// The result of planning a write: the sequence's new vertical data and the envelope of spec section 7.2.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Planned {
    /// The sequence's `vertical` after the write; the set as it was when nothing changed.
    pub set: Option<Box<VerticalSet>>,
    /// False on a no-op: the engine then records no step.
    pub changed: bool,
    /// The plan the write names; for `vertical.create` the new plan; None for a sequence write.
    pub plan: Option<u32>,
    /// The named plan's position ids added, changed and removed (empty for a sequence write; see `plans`).
    pub added: Vec<u32>,
    pub updated: Vec<u32>,
    pub removed: Vec<u32>,
    /// `vertical.deletePositions`: the ids the plan does not hold.
    pub missing: Vec<u32>,
    /// The id of each item an `add` named, in call order, new or matched (parts, regions, annotations); the new plan's id.
    pub ids: Vec<u32>,
    /// The named plan's changed frames, merged and half-open (spec section 7.2).
    pub changed_frames: Vec<[i64; 2]>,
    /// Every plan whose positions or frames the write changed, in plan order (a wall or part write can change several).
    pub plans: Vec<PlanDelta>,
    pub clamped: Vec<Clamped>,
    /// The issues touching the changed frames and every error-level issue of the plans written.
    pub issues: Vec<Issue>,
    /// What else the write did: moves shortened at a new wall, positions carried with a wall, repair steps.
    pub changes: Vec<Change>,
    /// The undo label ("Vertical: new plan 9:16"); empty on a no-op.
    pub label: String,
}

/// The most plans a set holds (I11).
pub const MAX_PLANS: usize = 16;

/// Bounds of the plan's and positions' text (I13, I20, I21).
const MAX_PLAN_NAME: usize = 120;
const MAX_LABEL: usize = 40;
const MAX_TAGS: usize = 16;
const MAX_TAG: usize = 64;
const MAX_NOTE: usize = 1_000;
const MAX_SUBJECTS: usize = 8;
const MAX_RISK: usize = 200;
const MAX_MARKS: usize = 2_000;

/// The work budget of `vertical.fitMove`'s search, in frames evaluated.
const FIT_BUDGET: u64 = 200_000_000;

/// Plans one write for `caller` (spec section 7.1, principle 3): the new vertical data of `seq` and what changed, or a
/// refusal with the numbers needed to fix the call. Pure; never panics on any input.
pub fn plan_write(project: &Project, seq: &Sequence, write: &Write, caller: Caller) -> Result<Planned, Refusal> {
    let ctx = Ctx { project, seq, before: seq.vertical.as_deref(), r: raster(seq), caller };
    let mut draft = Draft::new(&ctx);
    match write {
        Write::Create(w) => create(&ctx, &mut draft, w)?,
        Write::Update(w) => update(&ctx, &mut draft, w)?,
        Write::Delete(w) => delete(&ctx, &mut draft, w)?,
        Write::SetPosition(w) => set_position(&ctx, &mut draft, w)?,
        Write::RetimePosition(w) => retime(&ctx, &mut draft, w)?,
        Write::DeletePositions(w) => delete_positions(&ctx, &mut draft, w)?,
        Write::ReplaceRange(w) => replace_range(&ctx, &mut draft, w)?,
        Write::SetArrival(w) => set_arrival(&ctx, &mut draft, w)?,
        Write::SmoothThrough(w) => smooth(&ctx, &mut draft, w)?,
        Write::FitMove(w) => fit_move(&ctx, &mut draft, w)?,
        Write::SwapWindows(w) => swap_windows(&ctx, &mut draft, w)?,
        Write::SetWalls(w) => {
            sequence_guard(&ctx, w.expect.as_deref(), walls_digest)?;
            walls_guard(&ctx, w.expect_walls.as_deref())?;
            set_invalid(&ctx)?;
            set_walls(&ctx, &mut draft, w)?;
        }
        Write::ApplyMeasurement(w) => {
            walls_guard(&ctx, w.expect_walls.as_deref())?;
            set_invalid(&ctx)?;
            apply_measurement(&ctx, &mut draft, w)?;
        }
        Write::SeedShots(w) => seed_shots(&ctx, &mut draft, w)?,
        Write::SetParts(w) => {
            sequence_guard(&ctx, w.expect.as_deref(), parts_digest)?;
            set_invalid(&ctx)?;
            set_parts(&ctx, &mut draft, w)?;
        }
        Write::SetRegions(w) => {
            sequence_guard(&ctx, w.expect.as_deref(), regions_digest)?;
            set_invalid(&ctx)?;
            set_regions(&ctx, &mut draft, w)?;
        }
        Write::MarkChecked(w) => mark_checked(&ctx, &mut draft, w)?,
        Write::Repair(w) => {
            plan_of(&ctx, w.plan)?;
            draft.plan = Some(w.plan);
            repair(&ctx, &mut draft, w)?;
        }
        Write::SetAnnotations(w) => {
            sequence_guard(&ctx, w.expect.as_deref(), data_digest)?;
            set_invalid(&ctx)?;
            set_annotations(&ctx, &mut draft, w)?;
        }
        Write::SetActiveArea(w) => {
            sequence_guard(&ctx, w.expect.as_deref(), data_digest)?;
            set_invalid(&ctx)?;
            set_active_area(&ctx, &mut draft, w)?;
        }
        Write::ApprovePart(w) => {
            guard(&ctx, w.plan, None, None)?;
            draft.plan = Some(w.plan);
            approve_part(&ctx, &mut draft, w)?;
        }
        Write::SetReview(w) => set_review(&ctx, &mut draft, w)?,
        Write::SetLock(w) => set_lock(&ctx, &mut draft, w)?,
        Write::FlagAttention(w) => flag_attention(&ctx, &mut draft, w)?,
        Write::ClearAttention(w) => clear_attention(&ctx, &mut draft, w)?,
    }
    finish(&ctx, draft)
}

/// The frame that starts at tick `t` on `rate`, or `bad_time {floorFrame, nearestFrame}` when `t` is not a frame start
/// (spec section 7.2).
pub fn frame_of_time(rate: FrameRate, t: Tick) -> Result<i64, Refusal> {
    let floor = rate.frame_at(t);
    let start = rate.tick_of(floor);
    if start == t {
        return Ok(floor);
    }
    let next = floor.saturating_add(1);
    let nearest = if t.0.saturating_sub(start.0) <= rate.tick_of(next).0.saturating_sub(t.0) { floor } else { next };
    let detail = json!({"floorFrame": floor, "nearestFrame": nearest});
    Err(Refusal::new("bad_time", "the time is not a frame start; pass a frame", detail))
}

/// What every write sees: the project, the sequence before the write, its raster and the caller.
pub(crate) struct Ctx<'a> {
    pub project: &'a Project,
    pub seq: &'a Sequence,
    pub before: Option<&'a VerticalSet>,
    pub r: Raster,
    /// Every M2 write is a person's ([`PERSON`]); the agent-only branches are M5's.
    pub caller: Caller,
}

impl Ctx<'_> {
    /// The timecode of a frame for labels: the sequence's start timecode plus the frame, at its rate.
    pub fn tc(&self, frame: i64) -> String {
        let settings = &self.seq.settings;
        format_timecode_frames(frame.saturating_add(self.seq.start_timecode), settings.frame_rate, settings.drop_frame)
    }

    /// `out_of_range {key, value}` unless `frame` is a frame of the sequence, `[0, end)`.
    pub fn frame_in(&self, key: &str, frame: i64) -> Result<(), Refusal> {
        if (0..self.r.end_frame).contains(&frame) {
            return Ok(());
        }
        let detail = json!({"key": key, "value": frame, "end": self.r.end_frame});
        Err(Refusal::new("out_of_range", format!("frame {frame} is not a frame of the sequence"), detail))
    }

    /// The scene before the write.
    fn scene(&self) -> Scene<'_> {
        Scene::new(self.project, self.seq, self.before)
    }

    /// The origin of what the caller creates: `manual` for a person's write, `agent` for an agent's.
    pub fn origin(&self) -> Origin {
        match self.caller.writer {
            Writer::Human => Origin::Manual,
            Writer::Agent => Origin::Agent,
        }
    }

    /// The origin of a wall the caller stores, promotes or moves (W8: a moved wall takes its mover's origin): `manual` for a
    /// person, `agent` for an agent.
    pub fn wall_origin(&self) -> WallOrigin {
        match self.caller.writer {
            Writer::Human => WallOrigin::Manual,
            Writer::Agent => WallOrigin::Agent,
        }
    }
}

/// The working copy of a write and what it reports.
pub(crate) struct Draft {
    pub set: VerticalSet,
    pub plan: Option<u32>,
    pub missing: Vec<u32>,
    pub ids: Vec<u32>,
    pub clamped: Vec<Clamped>,
    pub changes: Vec<Change>,
    /// Write-specific issues (`wall_blocked_by_locked`).
    pub issues: Vec<Issue>,
    pub label: String,
    /// The positions whose flags the write sets itself (`MoveShortened`): they are not cleared.
    pub flagged: Vec<(u32, u32)>,
    /// A person's direct edit of positions: one changed position becomes reviewed (spec section 5.10).
    pub direct: bool,
    /// The positions a direct edit changes only as a by-product of the one it names (the arriving piece of a split, the
    /// next piece of a retimed split): the reviewed rule of spec section 5.10 does not count them.
    pub side: Vec<(u32, u32)>,
    /// The write may run on a plan that breaks an invariant (a person's marks, and repair): it is refused only for the
    /// violations it adds, and such a plan is not evaluated (spec section 2.8).
    pub lenient: bool,
    /// `strict`: the frames the changed frames must stay inside.
    pub strict: Option<[i64; 2]>,
}

impl Draft {
    fn new(ctx: &Ctx) -> Self {
        let set = ctx.before.cloned().unwrap_or_else(empty_set);
        Self {
            set,
            plan: None,
            missing: Vec::new(),
            ids: Vec::new(),
            clamped: Vec::new(),
            changes: Vec::new(),
            issues: Vec::new(),
            label: String::new(),
            flagged: Vec::new(),
            direct: false,
            side: Vec::new(),
            lenient: false,
            strict: None,
        }
    }

    /// The working copy of plan `id`.
    fn plan_mut(&mut self, id: u32) -> Result<&mut VerticalPlan, Refusal> {
        self.set.plans.iter_mut().find(|p| p.id == id).ok_or_else(|| not_found_plan(id))
    }
}

/// A set with no plan and no data, using edit points: what the first plan, part or wall write starts from.
pub(crate) fn empty_set() -> VerticalSet {
    VerticalSet {
        plans: Vec::new(),
        walls: WallSet { use_edit_points: true, stored: Vec::new(), suppressed: Vec::new(), candidates: Vec::new(), analysed: Vec::new() },
        parts: Vec::new(),
        regions: Vec::new(),
        annotations: Vec::new(),
        box_tracks: Vec::new(),
        active_area: Vec::new(),
        agent_overwrite: None,
        part_audio: PartAudio::default(),
        delivery: DeliveryInfo::default(),
        reference: None,
        deliveries: Vec::new(),
        next_plan_id: 1,
        next_part_id: 1,
        next_region_id: 1,
        next_annotation_id: 1,
    }
}

/// `too_many {key, count, bound}`.
pub(crate) fn too_many(key: &str, count: usize, bound: usize) -> Refusal {
    Refusal::new("too_many", format!("{key}: {count} is more than {bound}"), json!({"key": key, "count": count, "bound": bound}))
}

/// `too_many` when a list or a text is longer than its bound.
pub(crate) fn check_len(key: &str, count: usize, bound: usize) -> Result<(), Refusal> {
    if count > bound { Err(too_many(key, count, bound)) } else { Ok(()) }
}

/// `too_many {key, count, bound}` when `text` has more than `bound` characters.
pub(crate) fn chars_within(key: &str, text: &str, bound: usize) -> Result<(), Refusal> {
    let count = text.chars().take(bound.saturating_add(1)).count();
    if count <= bound {
        return Ok(());
    }
    Err(too_many(key, text.chars().count(), bound))
}

/// `bad_number {key}`: a number that is not finite.
pub(crate) fn bad_number(key: &str) -> Refusal {
    Refusal::new("bad_number", format!("{key} is not a finite number"), json!({"key": key}))
}

/// `bad_wall {frame}`.
pub(crate) fn bad_wall(frame: i64, why: &str) -> Refusal {
    Refusal::new("bad_wall", format!("frame {frame}: {why}"), json!({"frame": frame}))
}

/// `not_found {key: frame}`.
pub(crate) fn not_found_frame(key: &str, frame: i64) -> Refusal {
    Refusal::new("not_found", format!("nothing to act on at frame {frame}"), json!({ key: frame }))
}

/// `not_found {plan}`.
pub(crate) fn not_found_plan(plan: u32) -> Refusal {
    Refusal::new("not_found", format!("the sequence has no plan {plan}"), json!({"plan": plan}))
}

/// `not_found {key: id}` for a position, part, region or annotation.
pub(crate) fn not_found_id(key: &str, id: u32) -> Refusal {
    Refusal::new("not_found", format!("there is no {key} {id}"), json!({ key: id }))
}

/// `locked {ids}`: people unlock first.
pub(crate) fn locked(ids: Vec<u32>) -> Refusal {
    Refusal::new("locked", "the write would change locked positions; unlock them first", json!({"ids": ids}))
}

/// `invalid_plan {violations}`: the data breaks an invariant; `vertical.repair` fixes it.
pub(crate) fn invalid_plan(violations: Vec<Violation>) -> Refusal {
    Refusal::new("invalid_plan", "the plan breaks an invariant; repair it first", json!({"violations": violations}))
}

/// Half-open frame ranges sorted and merged (touching ranges join).
pub(crate) fn merge_ranges(mut ranges: Vec<[i64; 2]>) -> Vec<[i64; 2]> {
    ranges.retain(|[a, b]| a < b);
    ranges.sort_unstable();
    let mut out: Vec<[i64; 2]> = Vec::with_capacity(ranges.len());
    for [a, b] in ranges {
        match out.last_mut() {
            Some(last) if a <= last[1] => last[1] = last[1].max(b),
            _ => out.push([a, b]),
        }
    }
    out
}

/// The next id of a counter: never reused, above every id in use. `too_many {key: "ids"}` once a counter runs out.
pub(crate) fn alloc(counter: &mut u32, top: u32) -> Result<u32, Refusal> {
    let id = (*counter).max(top.saturating_add(1)).max(1);
    let Some(next) = id.checked_add(1) else { return Err(too_many("ids", u32::MAX as usize, u32::MAX as usize - 1)) };
    *counter = next;
    Ok(id)
}

/// The plan `id` as it was before the write: `not_found {plan}`.
fn plan_of<'a>(ctx: &Ctx<'a>, id: u32) -> Result<&'a VerticalPlan, Refusal> {
    ctx.before.and_then(|s| s.plans.iter().find(|p| p.id == id)).ok_or_else(|| not_found_plan(id))
}

/// The guards of a write that names a plan: it exists (`not_found`), `expect` is its digest (`stale_plan`),
/// `expect_range` is the range digest (`stale_range`), and it breaks no invariant (`invalid_plan`).
fn guard<'a>(ctx: &Ctx<'a>, id: u32, expect: Option<&str>, range: Option<&RangeGuard>) -> Result<&'a VerticalPlan, Refusal> {
    let plan = plan_of(ctx, id)?;
    if let Some(expected) = expect {
        let digest = plan_digest(ctx.seq, plan);
        if digest != expected {
            return Err(Refusal::new("stale_plan", "the plan changed since it was read", json!({"digest": digest})));
        }
    }
    let scene = ctx.scene();
    if let Some(g) = range {
        if g.from_frame < 0 || g.from_frame >= g.to_frame || g.to_frame > ctx.r.end_frame {
            let detail = json!({"key": "expectRange", "fromFrame": g.from_frame, "toFrame": g.to_frame, "end": ctx.r.end_frame});
            return Err(Refusal::new("out_of_range", "expectRange covers frames of the sequence, from before to", detail));
        }
        let digest = range_digest_in(&scene, plan, g.from_frame, g.to_frame);
        if digest != g.digest {
            let detail = json!({"fromFrame": g.from_frame, "toFrame": g.to_frame, "digest": digest});
            return Err(Refusal::new("stale_range", "the range changed since it was read", detail));
        }
    }
    let violations = plan_violations(&scene, plan);
    if !violations.is_empty() {
        return Err(invalid_plan(violations));
    }
    Ok(plan)
}

/// The guard of a person's marks (`setLock`, `setReview` without an opener, `flagAttention`, `clearAttention`): the plan
/// exists. They change no framing, so they work on a plan that breaks an invariant too: repair refuses `locked` for a
/// locked position that breaks one, and the person unlocks it (spec section 2.8). `finish` refuses only the violations
/// such a write adds.
fn marks_guard<'a>(ctx: &Ctx<'a>, draft: &mut Draft, id: u32) -> Result<&'a VerticalPlan, Refusal> {
    draft.lenient = true;
    plan_of(ctx, id)
}

/// `stale_plan {digest}` when a sequence write's `expect` is not the current digest of what it guards.
fn sequence_guard(ctx: &Ctx, expect: Option<&str>, digest: fn(&VerticalSet) -> String) -> Result<(), Refusal> {
    let Some(expected) = expect else { return Ok(()) };
    let current = digest(ctx.before.unwrap_or(&empty_set()));
    if current == expected {
        return Ok(());
    }
    Err(Refusal::new("stale_plan", "the data changed since it was read", json!({"digest": current})))
}

/// `stale_walls {digest}` when `expectWalls` is not the current effective-walls digest.
fn walls_guard(ctx: &Ctx, expect: Option<&str>) -> Result<(), Refusal> {
    let Some(expected) = expect else { return Ok(()) };
    let current = effective_walls_digest_in(&ctx.scene());
    if current == expected {
        return Ok(());
    }
    Err(Refusal::new("stale_walls", "the walls changed since they were read", json!({"digest": current})))
}

/// `invalid_plan` when any plan or the set's data breaks an invariant (the sequence writes, which touch every plan).
fn set_invalid(ctx: &Ctx) -> Result<(), Refusal> {
    let violations = validate(ctx.project, ctx.seq);
    if violations.is_empty() { Ok(()) } else { Err(invalid_plan(violations)) }
}

/// A plan's output size checked: both sides even and at least 2, inside the frame-size bounds (`invalid_output_size`).
fn check_output(width: Option<u32>, height: Option<u32>) -> Result<OutputSize, Refusal> {
    let refuse = || Refusal::new("invalid_output_size", "an output has two even sides of at least 2 px", json!({"width": width, "height": height}));
    let (Some(w), Some(h)) = (width, height) else { return Err(refuse()) };
    let even = w >= 2 && h >= 2 && w.is_multiple_of(2) && h.is_multiple_of(2);
    if !even || validate_frame_size(w, h).is_err() {
        return Err(refuse());
    }
    Ok(OutputSize { width: w, height: h })
}

/// `size_limit {minHeight, maxHeight}` when an output would need more than 4x upscale on the raster.
fn check_limits(r: &Raster, out: OutputSize) -> Result<Limits, Refusal> {
    let heights = limits(r, out);
    if heights.min_height > heights.max_height {
        let detail = json!({"minHeight": heights.min_height, "maxHeight": heights.max_height});
        return Err(Refusal::new("size_limit", "the output needs more than 4x upscale of the sequence", detail));
    }
    Ok(heights)
}

/// The reduced ratio of an output: "9:16".
fn ratio_name(out: OutputSize) -> String {
    let (mut a, mut b) = (out.width.max(1), out.height.max(1));
    while b != 0 {
        (a, b) = (b, a % b);
    }
    format!("{}:{}", out.width / a.max(1), out.height / a.max(1))
}

/// A plan name checked: 1 to 120 characters.
fn check_plan_name(name: &str) -> Result<(), Refusal> {
    chars_within("name", name, MAX_PLAN_NAME)?;
    if name.is_empty() {
        return Err(Refusal::new("out_of_range", "a plan has a name", json!({"key": "name"})));
    }
    Ok(())
}

/// `vertical.create`.
fn create(ctx: &Ctx, draft: &mut Draft, w: &CreatePlan) -> Result<(), Refusal> {
    if let Some(name) = &w.name {
        check_plan_name(name)?;
    }
    let par = ctx.seq.settings.par;
    if par.0 != par.1 {
        let detail = json!({"par": [par.0, par.1]});
        return Err(Refusal::new("unsupported_pixel_aspect", "the Vertical Edit needs square pixels", detail));
    }
    let output = match (w.width, w.height) {
        (None, None) => w.preset.unwrap_or_default().size(),
        (width, height) => check_output(width, height)?,
    };
    check_limits(&ctx.r, output)?;
    let name = w.name.clone().unwrap_or_else(|| ratio_name(output));
    if let Some(other) = draft.set.plans.iter().find(|p| p.name == name) {
        return Err(Refusal::new("plan_exists", format!("a plan is named {name:?} already"), json!({"id": other.id})));
    }
    if draft.set.plans.len() >= MAX_PLANS {
        return Err(too_many("plans", draft.set.plans.len() + 1, MAX_PLANS));
    }
    let top = draft.set.plans.iter().map(|p| p.id).max().unwrap_or(0);
    let id = alloc(&mut draft.set.next_plan_id, top)?;
    let mut plan = VerticalPlan::new(id, &name, output);
    plan.filter = w.filter.unwrap_or_default();
    draft.set.plans.push(plan);
    draft.plan = Some(id);
    draft.ids.push(id);
    draft.label = format!("Vertical: new plan {name}");
    Ok(())
}

/// `vertical.delete`.
fn delete(ctx: &Ctx, draft: &mut Draft, w: &DeletePlan) -> Result<(), Refusal> {
    let plan = plan_of(ctx, w.plan)?;
    if let Some(expected) = &w.expect {
        let digest = plan_digest(ctx.seq, plan);
        if digest != *expected {
            return Err(Refusal::new("stale_plan", "the plan changed since it was read", json!({"digest": digest})));
        }
    }
    draft.set.plans.retain(|p| p.id != w.plan);
    draft.plan = Some(w.plan);
    draft.label = format!("Vertical: delete plan {}", plan.name);
    Ok(())
}

/// A window a caller gave checked for numbers: `bad_number` when not finite, `out_of_range` beyond `MAX_COORDINATE`.
fn check_coordinates(key: &str, w: &Window) -> Result<(), Refusal> {
    for (name, v) in [("cx", w.cx), ("cy", w.cy), ("h", w.h)] {
        if !v.is_finite() {
            return Err(bad_number(if key.is_empty() { name } else { key }));
        }
        if v.abs() > MAX_COORDINATE {
            let detail = json!({"key": if key.is_empty() { name } else { key }, "value": v, "maximum": MAX_COORDINATE});
            return Err(Refusal::new("out_of_range", "a coordinate lies beyond 65 536", detail));
        }
    }
    Ok(())
}

/// The keys of a position checked before anything is planned (spec section 7.2): numbers finite and in range, lists and
/// texts within their bounds, curves valid, the focus and the confidence on their grids and inside their bounds.
fn check_fields(r: &Raster, f: &PositionFields) -> Result<(), Refusal> {
    for (key, v) in [("cx", f.cx), ("cy", f.cy), ("h", f.h)] {
        let Some(v) = v else { continue };
        if !v.is_finite() {
            return Err(bad_number(key));
        }
        if v.abs() > MAX_COORDINATE {
            let detail = json!({"key": key, "value": v, "maximum": MAX_COORDINATE});
            return Err(Refusal::new("out_of_range", format!("{key} lies beyond 65 536"), detail));
        }
    }
    if let Some(zoom) = f.zoom {
        if !zoom.is_finite() {
            return Err(bad_number("zoom"));
        }
        if zoom <= 0.0 || f.h.is_some() {
            return Err(Refusal::new("out_of_range", "zoom is a positive percentage, never given with h", json!({"key": "zoom", "value": zoom})));
        }
    }
    if let Some(frames) = f.move_frames {
        if frames < 2 {
            let detail = json!({"frames": frames, "minimum": 2});
            return Err(Refusal::new("move_too_short", "a move has at least 2 frames", detail));
        }
        if frames > i64::from(VE_MAX_FRAMES) {
            let detail = json!({"key": "moveFrames", "value": frames, "maximum": VE_MAX_FRAMES});
            return Err(Refusal::new("out_of_range", "a move has at most 1 000 000 frames", detail));
        }
    }
    if let Some(curve) = &f.curve {
        check_curve(curve)?;
    }
    if let Some(label) = &f.label {
        chars_within("label", label, MAX_LABEL)?;
    }
    if let Some(tags) = &f.tags {
        check_len("tags", tags.len(), MAX_TAGS)?;
        for (index, tag) in tags.iter().enumerate() {
            chars_within("tag", tag, MAX_TAG)?;
            if tag.is_empty() || tag.chars().any(char::is_control) {
                return Err(Refusal::new("out_of_range", "a tag has 1 to 64 characters, none a control", json!({"key": "tags", "index": index})));
            }
        }
    }
    if let Some(note) = &f.note {
        chars_within("note", note, MAX_NOTE)?;
    }
    if let Some(subjects) = &f.subjects {
        check_len("subjects", subjects.len(), MAX_SUBJECTS)?;
        if let Some(bad) = subjects.iter().find(|s| !valid_track_id(s)) {
            let detail = json!({"id": bad, "reason": "a box-track id is 1 to 64 characters of [A-Za-z0-9_:.-]"});
            return Err(Refusal::new("bad_box_track", "a subject is a box-track id", detail));
        }
    }
    if let Some(risk) = &f.risk {
        chars_within("risk", risk, MAX_RISK)?;
    }
    if let Some(Some([x, y])) = f.focus {
        if !x.is_finite() || !y.is_finite() {
            return Err(bad_number("focus"));
        }
        let (qx, qy) = (q(x), q(y));
        if qx < 0.0 || qy < 0.0 || qx > f64::from(r.width) || qy > f64::from(r.height) {
            return Err(Refusal::new("outside_picture", "the focus lies inside the frame", json!({"key": "focus", "focus": [qx, qy]})));
        }
    }
    if let Some(Some(c)) = f.confidence {
        if !c.is_finite() {
            return Err(bad_number("confidence"));
        }
        if !(0.0..=1.0).contains(&c) {
            return Err(Refusal::new("out_of_range", "a confidence lies in [0, 1]", json!({"key": "confidence", "value": c})));
        }
    }
    Ok(())
}

/// The window the keys give over `base` (omitted components are the base's; `zoom` converts as spec section 2.6), not yet
/// on the grid.
fn window_from(r: &Raster, out: OutputSize, base: Window, f: &PositionFields) -> Result<Window, Refusal> {
    let h = match (f.h, f.zoom) {
        (Some(h), _) => h,
        (None, Some(zoom)) => {
            let h = h_from_zoom(limits(r, out).max_height, zoom).filter(|h| h.is_finite() && *h <= MAX_COORDINATE);
            h.ok_or_else(|| Refusal::new("out_of_range", "the zoom gives no usable height", json!({"key": "zoom", "value": zoom})))?
        }
        (None, None) => base.h,
    };
    Ok(Window { cx: f.cx.unwrap_or(base.cx), cy: f.cy.unwrap_or(base.cy), h })
}

/// Stores a caller's window: with `clamp` the directed rounding of spec section 2.6 (reported in `clamped` when it moved
/// the window), else `q` of each value and the checks I4 (`size_limit`) and I3 (`outside_picture`).
fn store_window(r: &Raster, out: OutputSize, raw: Window, clamp: bool, at: (u32, i64), clamped: &mut Vec<Clamped>) -> Result<Window, Refusal> {
    let (position, frame) = at;
    if clamp {
        let w = clamp_window(raw, r, out);
        if w != quantize_window(raw) {
            clamped.push(Clamped { position, dx: w.cx - raw.cx, dy: w.cy - raw.cy, dh: w.h - raw.h });
        }
        return Ok(w);
    }
    let w = quantize_window(raw);
    check_window(r, out, &w, frame)?;
    Ok(w)
}

/// I4 then I3 on a stored window: `size_limit {minHeight, maxHeight}`, `outside_picture {frame, cx, cy}` with the legal
/// centres on the grid at that height.
fn check_window(r: &Raster, out: OutputSize, w: &Window, frame: i64) -> Result<(), Refusal> {
    let heights = limits(r, out);
    if w.h < heights.min_height || w.h > heights.max_height {
        let detail = json!({"minHeight": heights.min_height, "maxHeight": heights.max_height, "frame": frame, "h": w.h});
        return Err(Refusal::new("size_limit", "the window's height lies outside the plan's limits", detail));
    }
    let rect = w.rect(out);
    let eps = 1e-6;
    let inside = rect.x >= -eps && rect.y >= -eps && rect.x + rect.w <= f64::from(r.width) + eps && rect.y + rect.h <= f64::from(r.height) + eps;
    if !inside {
        let bounds = centre_bounds(r, out, w.h);
        let detail = json!({"frame": frame, "cx": bounds.cx, "cy": bounds.cy, "rect": [rect.x, rect.y, rect.w, rect.h]});
        return Err(Refusal::new("outside_picture", "the window would leave the picture; pass clamp to move it inward", detail));
    }
    Ok(())
}

/// The arrival the keys give over `current` (spec section 7.2): `moveFrames` or `curve` alone mean a move; a move written
/// without a curve is Ramp 20/20 when it is new and keeps its curve when it was a move; the same length and curve keep a
/// split piece's `splitOf`. A new move needs `moveFrames`; a cut takes neither key.
fn arrival_from(current: Option<Arrival>, f: &PositionFields) -> Result<Arrival, Refusal> {
    let kind = f.arrival.or((f.move_frames.is_some() || f.curve.is_some()).then_some(ArrivalKind::Move));
    match kind {
        None => Ok(current.unwrap_or(Arrival::Cut)),
        Some(ArrivalKind::Cut) => {
            if f.move_frames.is_some() || f.curve.is_some() {
                let detail = json!({"key": "moveFrames", "arrival": "cut"});
                return Err(Refusal::new("out_of_range", "a cut takes no moveFrames and no curve", detail));
            }
            Ok(Arrival::Cut)
        }
        Some(ArrivalKind::Move) => {
            let (frames, curve, split) = match current {
                Some(Arrival::Move { move_frames, curve, split_of }) => (Some(move_frames), Some(curve), split_of),
                _ => (None, None, None),
            };
            let given = f.move_frames.and_then(|d| u32::try_from(d).ok());
            let Some(d) = given.or(frames) else {
                let detail = json!({"key": "moveFrames", "minimum": 2, "maximum": VE_MAX_FRAMES});
                return Err(Refusal::new("out_of_range", "a new move needs moveFrames", detail));
            };
            let c = f.curve.or(curve).unwrap_or(DEFAULT_CURVE);
            let split_of = (Some(d) == frames && Some(c) == curve).then_some(split).flatten();
            Ok(Arrival::Move { move_frames: d, curve: c, split_of })
        }
    }
}

/// The metadata keys applied to a position: an omitted key keeps, an explicit null clears.
fn apply_metadata(p: &mut Position, f: &PositionFields) {
    if let Some(anchor) = f.caption_anchor {
        p.caption_anchor = anchor;
    }
    if let Some(label) = &f.label {
        p.label = label.clone();
    }
    if let Some(color) = f.color {
        p.color = color;
    }
    if let Some(tags) = &f.tags {
        p.tags = tags.clone();
    }
    if let Some(note) = &f.note {
        p.note = note.clone();
    }
    if let Some(subjects) = &f.subjects {
        p.subjects = subjects.clone();
    }
    if let Some(focus) = f.focus {
        p.focus = focus.map(|[x, y]| [q(x), q(y)]);
    }
    if let Some(confidence) = f.confidence {
        p.confidence = confidence.map(q);
    }
    if let Some(risk) = &f.risk {
        p.risk = risk.clone();
    }
}

/// Whether a window component, `h` or `zoom` is among the keys.
fn names_window(f: &PositionFields) -> bool {
    f.cx.is_some() || f.cy.is_some() || f.h.is_some() || f.zoom.is_some()
}

/// The next position id of a plan.
fn next_position(plan: &mut VerticalPlan) -> Result<u32, Refusal> {
    let top = plan.positions.iter().map(|p| p.id).max().unwrap_or(0);
    alloc(&mut plan.next_position_id, top)
}

/// `vertical.setPosition` (spec sections 3.6 and 7.3): upsert by frame; inside a move the exact split first.
fn set_position(ctx: &Ctx, draft: &mut Draft, w: &SetPosition) -> Result<(), Refusal> {
    let before = guard(ctx, w.plan, w.expect.as_deref(), w.expect_range.as_ref())?;
    ctx.frame_in("frame", w.frame)?;
    check_fields(&ctx.r, &w.fields)?;
    let sources = usize::from(w.from_label.is_some()) + usize::from(w.from_position.is_some());
    if sources > 1 || (sources == 1 && names_window(&w.fields)) {
        let detail = json!({"key": "fromLabel", "reason": "fromLabel, fromPosition and a window exclude each other"});
        return Err(Refusal::new("out_of_range", "pass one of fromLabel, fromPosition or a window", detail));
    }
    let scene = ctx.scene();
    let r = ctx.r;
    let out = before.output;
    let source = match (&w.from_label, w.from_position) {
        (Some(label), _) => Some(from_label_in(&scene, before, w.frame, label).map(|s| (s.window, s.layout, Some(label.clone())))?),
        (None, Some(id)) => {
            let p = before.positions.iter().find(|p| p.id == id).ok_or_else(|| not_found_id("position", id))?;
            Some((p.window, p.layout, None))
        }
        (None, None) => None,
    };
    let at = r.rate.tick_of(w.frame);
    let existing = before.positions.iter().position(|p| p.at == at);
    let split = if existing.is_none() { split_move(before, &r, w.frame) } else { None };
    let shown = window_at(before, &r, at);
    draft.plan = Some(w.plan);
    draft.direct = true;
    // the arriving piece of a split changes only as a by-product of the waypoint the person edits
    let mut side = None;
    let plan = draft.plan_mut(w.plan)?;
    let (index, base, base_layout, kind) = match (existing, split) {
        (Some(k), _) => {
            let p = plan.positions.get(k).ok_or_else(|| not_found_plan(w.plan))?;
            (k, p.window, p.layout, "move window")
        }
        (None, Some(split)) => {
            let arriving = plan.positions.get(split.index).ok_or_else(|| not_found_plan(w.plan))?;
            if arriving.locked {
                return Err(locked(vec![arriving.id]));
            }
            side = Some(arriving.id);
            let agreed = arrival_from(Some(split.arrival), &w.fields).is_ok_and(|a| a == split.arrival);
            if (w.fields.arrival.is_some() || w.fields.move_frames.is_some() || w.fields.curve.is_some()) && !agreed {
                let detail = json!({"arrival": split.arrival});
                return Err(Refusal::new("split_conflict", "the given arrival disagrees with the exact split of the move", detail));
            }
            let id = next_position(plan)?;
            insert_split(plan, &split, id, ctx.origin(), ctx.caller.writer);
            (split.index, split.window, Layout::Fill, "waypoint")
        }
        (None, None) => {
            let id = next_position(plan)?;
            let index = plan.positions.partition_point(|p| p.at < at);
            plan.positions.insert(index, Position::new(id, at, shown.window, Arrival::Cut, ctx.origin(), ctx.caller.writer));
            (index, shown.window, shown.layout, "new position")
        }
    };
    let Some(p) = plan.positions.get_mut(index) else { return Err(not_found_plan(w.plan)) };
    let id = p.id;
    let (window, layout) = match &source {
        Some((window, layout, _)) => (*window, *layout),
        None => (window_from(&r, out, base, &w.fields)?, w.fields.layout.unwrap_or(base_layout)),
    };
    let given = source.is_some() || names_window(&w.fields);
    let mut clamped = Vec::new();
    p.window = if given { store_window(&r, out, window, w.clamp, (id, w.frame), &mut clamped)? } else { window };
    p.layout = layout;
    if kind != "waypoint" {
        p.arrival = arrival_from(Some(p.arrival), &w.fields)?;
    }
    apply_metadata(p, &w.fields);
    if let Some((_, _, Some(label))) = &source
        && w.fields.label.is_none()
    {
        p.label = label.clone();
    }
    draft.clamped = clamped;
    draft.side.extend(side.map(|arriving| (w.plan, arriving)));
    draft.label = match &w.from_label {
        Some(label) => format!("Vertical: apply {label} (#{id})"),
        None => format!("Vertical: {kind} (#{id})"),
    };
    Ok(())
}

/// `vertical.retimePosition`: moves a position in time, keeping its id, window and move length; the moves it ends and
/// starts become whole moves.
fn retime(ctx: &Ctx, draft: &mut Draft, w: &RetimePosition) -> Result<(), Refusal> {
    let before = guard(ctx, w.plan, w.expect.as_deref(), w.expect_range.as_ref())?;
    ctx.frame_in("frame", w.frame)?;
    let rate = ctx.r.rate;
    let k = before.positions.iter().position(|p| p.id == w.position).ok_or_else(|| not_found_id("position", w.position))?;
    let lo = k.checked_sub(1).and_then(|j| before.positions.get(j)).map_or(0, |p| rate.frame_at(p.at).saturating_add(1));
    let hi = before.positions.get(k + 1).map_or(ctx.r.end_frame, |p| rate.frame_at(p.at)).saturating_sub(1);
    if w.frame < lo || w.frame > hi {
        let detail = json!({"position": w.position, "frame": w.frame, "allowed": [lo, hi]});
        return Err(Refusal::new("would_reorder", "a position moves only between its neighbours", detail));
    }
    draft.plan = Some(w.plan);
    draft.direct = true;
    // the next piece of a split becomes a whole move only as a by-product of the position the person retimes
    let mut side = None;
    let plan = draft.plan_mut(w.plan)?;
    for j in [k, k + 1] {
        if let Some(p) = plan.positions.get_mut(j)
            && let Arrival::Move { move_frames, curve, split_of: Some(_) } = p.arrival
        {
            p.arrival = Arrival::Move { move_frames, curve, split_of: None };
            side = side.or((j != k).then_some(p.id));
        }
    }
    if let Some(p) = plan.positions.get_mut(k) {
        p.at = rate.tick_of(w.frame);
    }
    draft.side.extend(side.map(|next| (w.plan, next)));
    draft.label = format!("Vertical: retime (#{})", w.position);
    Ok(())
}

/// `vertical.deletePositions`: missing ids are listed, not refused.
fn delete_positions(ctx: &Ctx, draft: &mut Draft, w: &DeletePositions) -> Result<(), Refusal> {
    check_len("positions", w.positions.len(), MAX_LIST)?;
    let before = guard(ctx, w.plan, w.expect.as_deref(), w.expect_range.as_ref())?;
    let wanted: BTreeSet<u32> = w.positions.iter().copied().collect();
    let present: BTreeSet<u32> = before.positions.iter().map(|p| p.id).collect();
    draft.missing = wanted.difference(&present).copied().collect();
    draft.plan = Some(w.plan);
    let plan = draft.plan_mut(w.plan)?;
    plan.positions.retain(|p| !wanted.contains(&p.id));
    let n = present.intersection(&wanted).count();
    draft.label = format!("Vertical: delete {n} position{}", plural(n));
    Ok(())
}

/// `vertical.replaceRange`: atomically replaces every position with a frame in `[from, to)`; matched frames keep their
/// ids and their omitted keys; a new frame needs `cx`, `cy` and `h` or `zoom`.
fn replace_range(ctx: &Ctx, draft: &mut Draft, w: &ReplaceRange) -> Result<(), Refusal> {
    check_len("positions", w.items.len(), MAX_REPLACE_ITEMS)?;
    let before = guard(ctx, w.plan, w.expect.as_deref(), w.expect_range.as_ref())?;
    walls_guard(ctx, w.expect_walls.as_deref())?;
    let (from, to) = (w.from_frame, w.to_frame);
    if from < 0 || from >= to || to > ctx.r.end_frame {
        let detail = json!({"key": "fromFrame", "fromFrame": from, "toFrame": to, "end": ctx.r.end_frame});
        return Err(Refusal::new("out_of_range", "the range lies inside the sequence, from before to", detail));
    }
    let mut items: Vec<(usize, &ReplaceItem)> = w.items.iter().enumerate().collect();
    for (index, item) in &items {
        if item.frame < from || item.frame >= to {
            let detail = json!({"key": "frame", "value": item.frame, "index": index});
            return Err(Refusal::new("out_of_range", "an item lies inside the replaced range", detail));
        }
        check_fields(&ctx.r, &item.fields)?;
    }
    items.sort_by_key(|(_, item)| item.frame);
    if let Some(pair) = items.windows(2).find(|p| p.first().map(|x| x.1.frame) == p.get(1).map(|x| x.1.frame)) {
        let frame = pair.first().map_or(0, |x| x.1.frame);
        return Err(Refusal::new("position_exists", format!("two items name frame {frame}"), json!({"frame": frame})));
    }
    let r = ctx.r;
    let rate = r.rate;
    let out = before.output;
    draft.plan = Some(w.plan);
    draft.direct = true;
    draft.strict = w.strict.then_some([from, to]);
    let mut clamped = Vec::new();
    let plan = draft.plan_mut(w.plan)?;
    let top = plan.positions.iter().map(|p| p.id).max().unwrap_or(0);
    let (inside, mut kept): (Vec<Position>, Vec<Position>) = std::mem::take(&mut plan.positions).into_iter().partition(|p| {
        let f = rate.frame_at(p.at);
        f >= from && f < to
    });
    for (index, item) in items {
        let at = rate.tick_of(item.frame);
        let matched = inside.iter().find(|p| p.at == at);
        let f = &item.fields;
        let mut p = match matched {
            Some(p) => p.clone(),
            None => {
                if f.cx.is_none() || f.cy.is_none() || (f.h.is_none() && f.zoom.is_none()) {
                    let detail = json!({"key": "cx", "index": index, "reason": "a new position needs cx, cy and h or zoom"});
                    return Err(Refusal::new("out_of_range", "a new position needs cx, cy and h or zoom", detail));
                }
                let id = alloc(&mut plan.next_position_id, top)?;
                Position::new(id, at, Window::default(), Arrival::Cut, ctx.origin(), ctx.caller.writer)
            }
        };
        let raw = window_from(&r, out, p.window, f)?;
        if names_window(f) {
            p.window = store_window(&r, out, raw, w.clamp, (p.id, item.frame), &mut clamped)?;
        }
        p.layout = f.layout.unwrap_or(p.layout);
        p.arrival = arrival_from(matched.map(|m| m.arrival), f)?;
        apply_metadata(&mut p, f);
        kept.push(p);
    }
    kept.sort_by_key(|p| p.at);
    plan.positions = kept;
    draft.clamped = clamped;
    let n = w.items.len();
    draft.label = format!("Vertical: replace {n} position{} {}\u{2013}{}", plural(n), ctx.tc(from), ctx.tc(to));
    Ok(())
}

/// `vertical.setArrival`: how the window arrives at each of several positions.
fn set_arrival(ctx: &Ctx, draft: &mut Draft, w: &SetArrival) -> Result<(), Refusal> {
    check_len("positions", w.positions.len(), MAX_LIST)?;
    let before = guard(ctx, w.plan, w.expect.as_deref(), w.expect_range.as_ref())?;
    let fields = PositionFields { arrival: Some(w.arrival), move_frames: w.move_frames, curve: w.curve, ..PositionFields::default() };
    check_fields(&ctx.r, &fields)?;
    if let Some(&missing) = w.positions.iter().find(|id| !before.positions.iter().any(|p| p.id == **id)) {
        return Err(not_found_id("position", missing));
    }
    draft.plan = Some(w.plan);
    draft.direct = true;
    let plan = draft.plan_mut(w.plan)?;
    let mut first: Option<Arrival> = None;
    for p in plan.positions.iter_mut().filter(|p| w.positions.contains(&p.id)) {
        p.arrival = arrival_from(Some(p.arrival), &fields)?;
        first = first.or(Some(p.arrival));
    }
    let what = match first {
        Some(Arrival::Move { move_frames, curve, .. }) => format!("{} {move_frames} f", curve_name(&curve).to_lowercase()),
        _ => "cut".to_string(),
    };
    draft.label = format!("Vertical: arrival {what} ({})", id_list(&w.positions));
    Ok(())
}

/// "#2" for one id, "#1, #2" for a few, "5 positions" for many.
fn id_list(ids: &[u32]) -> String {
    match ids {
        [one] => format!("#{one}"),
        few if few.len() <= 4 => few.iter().map(|id| format!("#{id}")).collect::<Vec<_>>().join(", "),
        many => format!("{} positions", many.len()),
    }
}

/// `vertical.smoothThrough` (spec section 3.7): the walls inside the run are refused first (`move_crosses_wall`), then
/// M1's `smooth_through` gives the arrivals.
fn smooth(ctx: &Ctx, draft: &mut Draft, w: &SmoothThrough) -> Result<(), Refusal> {
    check_len("positions", w.positions.len(), MAX_LIST)?;
    let before = guard(ctx, w.plan, w.expect.as_deref(), w.expect_range.as_ref())?;
    let rate = ctx.r.rate;
    let index: Vec<Option<usize>> = w.positions.iter().map(|id| before.positions.iter().position(|p| p.id == *id)).collect();
    if let Some(missing) = w.positions.iter().zip(&index).find(|(_, i)| i.is_none()).map(|(id, _)| *id) {
        return Err(not_found_id("position", missing));
    }
    let first = index.first().copied().flatten().unwrap_or(0);
    let consecutive = index.iter().enumerate().all(|(k, i)| *i == first.checked_add(k));
    if w.positions.len() < 3 || !consecutive {
        let detail = json!({"count": w.positions.len(), "minimum": 3});
        return Err(Refusal::new("out_of_range", "Smooth through needs at least 3 consecutive positions", detail));
    }
    let scene = ctx.scene();
    let run = before.positions.get(first..first + w.positions.len()).unwrap_or_default();
    for pair in run.windows(2) {
        let [a, b] = pair else { continue };
        let (fa, fb) = (rate.frame_at(a.at), rate.frame_at(b.at));
        if let Some(&wall) = scene.wall_frames.iter().find(|&&c| c > fa && c <= fb) {
            return Err(crosses(&scene, b.id, fb, wall));
        }
    }
    let arrivals = smooth_through(before, &ctx.r, &w.positions, w.opts)?;
    draft.plan = Some(w.plan);
    draft.direct = true;
    let plan = draft.plan_mut(w.plan)?;
    for (p, arrival) in plan.positions.iter_mut().skip(first + 1).zip(arrivals) {
        p.arrival = arrival;
    }
    let (a, b) = (w.positions.first().copied().unwrap_or(0), w.positions.last().copied().unwrap_or(0));
    draft.label = format!("Vertical: smooth through #{a}\u{2013}#{b}");
    Ok(())
}

/// `move_crosses_wall {position, frame, wall, sources, maxMoveFrames}` for a move into `position` (frame `frame`).
fn crosses(scene: &Scene, position: u32, frame: i64, wall: i64) -> Refusal {
    let sources = scene.wall_at(wall).map(|w| w.sources.clone()).unwrap_or_default();
    let detail = json!({"position": position, "frame": frame, "wall": wall, "sources": sources, "maxMoveFrames": frame - wall});
    Refusal::new("move_crosses_wall", "a move lives in one shot: it may start on a wall, never contain one", detail)
}

/// `vertical.fitMove` (spec section 3.8): the shortest move into the position whose measured peak speed (and acceleration,
/// when given) meets the limits, over its frames plus one frame of rest on each side.
fn fit_move(ctx: &Ctx, draft: &mut Draft, w: &FitMove) -> Result<(), Refusal> {
    let before = guard(ctx, w.plan, w.expect.as_deref(), w.expect_range.as_ref())?;
    if !w.max_speed.is_finite() || w.max_speed <= 0.0 {
        return Err(bad_number("maxSpeed"));
    }
    if w.max_accel.is_some_and(|a| !a.is_finite() || a <= 0.0) {
        return Err(bad_number("maxAccel"));
    }
    if let Some(curve) = &w.curve {
        check_curve(curve)?;
    }
    let rate = ctx.r.rate;
    let k = before.positions.iter().position(|p| p.id == w.position).ok_or_else(|| not_found_id("position", w.position))?;
    let (Some(departing), Some(arriving)) = (k.checked_sub(1).and_then(|j| before.positions.get(j)), before.positions.get(k)) else {
        let detail = json!({"key": "position", "position": w.position, "reason": "the first position has nothing to move from"});
        return Err(Refusal::new("out_of_range", "the first position has no move", detail));
    };
    if arriving.locked {
        return Err(locked(vec![arriving.id]));
    }
    if departing.layout == Layout::Fit || arriving.layout == Layout::Fit {
        return Err(Refusal::new("layout_move", "a move never arrives at or leaves a fit position", json!({"ids": [arriving.id]})));
    }
    let curve = match (w.curve, arriving.arrival) {
        (Some(c), _) => c,
        (None, Arrival::Move { curve, .. }) => curve,
        (None, Arrival::Cut) => DEFAULT_CURVE,
    };
    let scene = ctx.scene();
    let (f, previous) = (rate.frame_at(arriving.at), rate.frame_at(departing.at));
    let start = scene.wall_frames.iter().copied().filter(|&c| c > previous && c <= f).max().unwrap_or(previous);
    let room = f.saturating_sub(start).max(0);
    let out = before.output;
    let (a, b) = (departing.window, arriving.window);
    let distance = corner_deltas(&a, &b, out).map_or(0.0, |d| speed_of(&d));
    let unmet = |peak: f64| {
        let detail = json!({"position": w.position, "maxMoveFrames": room, "peak": peak, "alternative": {"arrival": "cut"}});
        Refusal::new("speed_limit_unmet", "no move that fits the room meets the limits; a cut reaches the same window", detail)
    };
    let room_frames = u32::try_from(room).unwrap_or(VE_MAX_FRAMES).min(VE_MAX_FRAMES);
    if room_frames < 2 {
        return Err(unmet(distance));
    }
    let longest = || whole_move_numbers(&a, &b, room_frames, &curve, out).peak_speed;
    // spec section 3.8 starts at ceil(D_out / maxSpeed), a lower bound for a pan or a zoom in. A zoom out measures each
    // frame against the larger height before it, so its frames' speeds sum to at least D_out * h_a / h_b only: that is its
    // bound. The search starts a hair below the bound, so float noise never hides a shorter move that holds (T-M10).
    let zoom_out = if b.h > a.h { a.h / b.h } else { 1.0 };
    let shortest = (distance * zoom_out / w.max_speed * (1.0 - 1e-9)).floor();
    if shortest.is_nan() || shortest > f64::from(room_frames) {
        return Err(unmet(longest()));
    }
    let mut budget = FIT_BUDGET;
    let mut found = None;
    for d in (shortest as u32).max(2)..=room_frames {
        match move_fits(&a, &b, d, &curve, out, (w.max_speed, w.max_accel), &mut budget) {
            Fits::Holds => {
                found = Some(d);
                break;
            }
            Fits::Fails => {}
            Fits::Spent => {
                let detail = json!({"key": "maxSpeed", "reason": "the search spent its budget", "maxMoveFrames": room});
                return Err(Refusal::new("out_of_range", "the move is too long to fit to this speed", detail));
            }
        }
    }
    let Some(d) = found else { return Err(unmet(longest())) };
    draft.plan = Some(w.plan);
    draft.direct = true;
    let plan = draft.plan_mut(w.plan)?;
    if let Some(p) = plan.positions.get_mut(k) {
        p.arrival = Arrival::Move { move_frames: d, curve, split_of: None };
    }
    draft.label = format!("Vertical: fit to speed {d} f (#{})", w.position);
    Ok(())
}

/// `vertical.swapWindows`: exchanges two positions' windows and layouts, keeping their times and arrivals.
fn swap_windows(ctx: &Ctx, draft: &mut Draft, w: &SwapWindows) -> Result<(), Refusal> {
    let [a, b] = w.positions.as_slice() else {
        let detail = json!({"key": "positions", "count": w.positions.len(), "required": 2});
        return Err(Refusal::new("out_of_range", "swapWindows takes exactly two positions", detail));
    };
    let before = guard(ctx, w.plan, w.expect.as_deref(), w.expect_range.as_ref())?;
    let find = |id: u32| before.positions.iter().find(|p| p.id == id).ok_or_else(|| not_found_id("position", id));
    let (pa, pb) = (find(*a)?, find(*b)?);
    let (wa, la, wb, lb) = (pa.window, pa.layout, pb.window, pb.layout);
    draft.plan = Some(w.plan);
    draft.direct = true;
    let plan = draft.plan_mut(w.plan)?;
    for p in &mut plan.positions {
        if p.id == *a {
            (p.window, p.layout) = (wb, lb);
        } else if p.id == *b {
            (p.window, p.layout) = (wa, la);
        }
    }
    draft.label = format!("Vertical: swap windows (#{a}, #{b})");
    Ok(())
}

/// `vertical.markChecked`: clears the flags of the positions (all of the plan's when none is named).
fn mark_checked(ctx: &Ctx, draft: &mut Draft, w: &MarkChecked) -> Result<(), Refusal> {
    if let Some(ids) = &w.positions {
        check_len("positions", ids.len(), MAX_LIST)?;
    }
    let before = guard(ctx, w.plan, w.expect.as_deref(), w.expect_range.as_ref())?;
    if let Some(&missing) = w.positions.iter().flatten().find(|id| !before.positions.iter().any(|p| p.id == **id)) {
        return Err(not_found_id("position", missing));
    }
    draft.plan = Some(w.plan);
    let plan = draft.plan_mut(w.plan)?;
    let named = |id: u32| w.positions.as_ref().is_none_or(|ids| ids.contains(&id));
    for p in plan.positions.iter_mut().filter(|p| named(p.id)) {
        p.flags.clear();
    }
    draft.label = match &w.positions {
        Some(ids) => format!("Vertical: mark checked ({})", id_list(ids)),
        None => "Vertical: mark checked".into(),
    };
    Ok(())
}

/// `vertical.seedShots` (spec section 4.5): an opening cut on every wall (and frame 0) in range that has none, with the
/// window already shown there (`carry`) or the plan's starting window inside the active area (`starting`).
fn seed_shots(ctx: &Ctx, draft: &mut Draft, w: &SeedShots) -> Result<(), Refusal> {
    let before = guard(ctx, w.plan, w.expect.as_deref(), None)?;
    let r = ctx.r;
    let (from, to) = (w.from_frame.unwrap_or(0), w.to_frame.unwrap_or(r.end_frame));
    if from < 0 || from >= to || to > r.end_frame {
        let detail = json!({"key": "fromFrame", "fromFrame": from, "toFrame": to, "end": r.end_frame});
        return Err(Refusal::new("out_of_range", "the range lies inside the sequence, from before to", detail));
    }
    let scene = ctx.scene();
    let shots: Vec<i64> = std::iter::once(0).chain(scene.wall_frames.iter().copied()).filter(|&f| f >= from && f < to).collect();
    let start = before.starting_window.unwrap_or_else(|| centred_largest(&r, before.output));
    let mut openers = Vec::new();
    for f in shots {
        let at = r.rate.tick_of(f);
        if before.positions.iter().any(|p| p.at == at) {
            continue;
        }
        let opener = match w.mode {
            SeedMode::Carry => {
                let shown = window_at(before, &r, at);
                (shown.window, shown.layout)
            }
            SeedMode::Starting => {
                let area = ctx.before.map_or([0.0, 0.0, f64::from(r.width), f64::from(r.height)], |set| active_rect(set, &r, at));
                (clamp_to_area(start, area, &r, before.output), Layout::Fill)
            }
        };
        openers.push((at, opener));
    }
    draft.plan = Some(w.plan);
    let mut changes = Vec::new();
    let plan = draft.plan_mut(w.plan)?;
    let first = plan.positions.first().map(|p| p.id);
    let n = openers.len();
    for (at, (window, layout)) in openers {
        let id = next_position(plan)?;
        let index = plan.positions.partition_point(|p| p.at < at);
        let p = Position { layout, ..Position::new(id, at, window, Arrival::Cut, Origin::Seeded, ctx.caller.writer) };
        plan.positions.insert(index, p);
    }
    settle_first_move(plan, &r, &scene.wall_frames, first, &mut changes);
    draft.changes.extend(changes);
    draft.label = format!("Vertical: seed {n} position{}", plural(n));
    Ok(())
}

/// Openers were added in front of what was the plan's first position (`first`): the move it holds, which had no effect
/// (spec section 2.5), now departs from the opener before it. When that move does not fit there (I5's room, I6's walls,
/// I9's layouts) it becomes a cut, which leaves every frame from that position on as it was, and `move_cut` reports it.
/// A locked position is not exempt: `finish` then refuses `locked`.
fn settle_first_move(plan: &mut VerticalPlan, r: &Raster, walls: &[i64], first: Option<u32>, changes: &mut Vec<Change>) {
    let Some(id) = first else { return };
    let Some(k) = plan.positions.iter().position(|p| p.id == id) else { return };
    let (Some(departing), Some(p)) = (k.checked_sub(1).and_then(|j| plan.positions.get(j)), plan.positions.get(k)) else { return };
    let Arrival::Move { move_frames, .. } = p.arrival else { return };
    let (f, previous) = (r.rate.frame_at(p.at), r.rate.frame_at(departing.at));
    let s = f.saturating_sub(i64::from(move_frames));
    let crosses = walls.partition_point(|&c| c <= s.max(previous)) < walls.partition_point(|&c| c <= f);
    if s >= previous && !crosses && departing.layout == Layout::Fill && p.layout == Layout::Fill {
        return;
    }
    let plan_id = plan.id;
    if let Some(p) = plan.positions.get_mut(k) {
        p.arrival = Arrival::Cut;
        changes.push(Change { code: "move_cut", detail: json!({"plan": plan_id, "position": id, "reason": "a position was added before it"}) });
    }
}

/// `vertical.setReview`: the review state of positions, or of a whole shot (Alt+R's one step: a `SAME` opener added when
/// the shot has none, every position reviewed, the shot's attention marks cleared).
fn set_review(ctx: &Ctx, draft: &mut Draft, w: &SetReview) -> Result<(), Refusal> {
    check_len("positions", w.positions.len(), MAX_LIST)?;
    // approving a shot may add its opener, which evaluates the plan: that needs a valid one
    let opens = w.reviewed && w.shot_frame.is_some();
    let before = if opens { guard(ctx, w.plan, None, None)? } else { marks_guard(ctx, draft, w.plan)? };
    if let Some(&missing) = w.positions.iter().find(|id| !before.positions.iter().any(|p| p.id == **id)) {
        return Err(not_found_id("position", missing));
    }
    let r = ctx.r;
    let scene = ctx.scene();
    let shot = match w.shot_frame {
        Some(frame) => {
            ctx.frame_in("shotFrame", frame)?;
            Some((scene.shot_index(frame) + 1, scene.shot_of(frame)))
        }
        None => None,
    };
    draft.plan = Some(w.plan);
    let mut changes = Vec::new();
    let plan = draft.plan_mut(w.plan)?;
    for p in plan.positions.iter_mut().filter(|p| w.positions.contains(&p.id)) {
        p.reviewed = w.reviewed;
    }
    let Some((number, [a, b])) = shot else {
        let what = if w.reviewed { "reviewed" } else { "unreviewed" };
        draft.label = format!("Vertical: mark {what} ({})", id_list(&w.positions));
        return Ok(());
    };
    let opener_at = r.rate.tick_of(a);
    if w.reviewed && !plan.positions.iter().any(|p| p.at == opener_at) {
        let shown = window_at(plan, &r, opener_at);
        let first = plan.positions.first().map(|p| p.id);
        let id = next_position(plan)?;
        let index = plan.positions.partition_point(|p| p.at < opener_at);
        let opener = Position::new(id, opener_at, shown.window, Arrival::Cut, ctx.origin(), ctx.caller.writer);
        plan.positions.insert(index, Position { layout: shown.layout, reviewed: true, ..opener });
        settle_first_move(plan, &r, &scene.wall_frames, first, &mut changes);
    }
    let in_shot = |t: Tick| (a..b).contains(&r.rate.frame_at(t));
    for p in plan.positions.iter_mut().filter(|p| in_shot(p.at)) {
        p.reviewed = w.reviewed;
    }
    if w.reviewed {
        plan.attention.retain(|m| !in_shot(m.at));
    }
    draft.changes.extend(changes);
    let what = if w.reviewed { "approve" } else { "unreview" };
    draft.label = format!("Vertical: {what} shot {number}");
    Ok(())
}

/// `vertical.setLock`.
fn set_lock(ctx: &Ctx, draft: &mut Draft, w: &SetLock) -> Result<(), Refusal> {
    check_len("positions", w.positions.len(), MAX_LIST)?;
    let before = marks_guard(ctx, draft, w.plan)?;
    if let Some(&missing) = w.positions.iter().find(|id| !before.positions.iter().any(|p| p.id == **id)) {
        return Err(not_found_id("position", missing));
    }
    draft.plan = Some(w.plan);
    let plan = draft.plan_mut(w.plan)?;
    for p in plan.positions.iter_mut().filter(|p| w.positions.contains(&p.id)) {
        p.locked = w.locked;
    }
    let ids = w.positions.iter().map(|id| format!("#{id}")).collect::<Vec<_>>().join(", ");
    draft.label = format!("Vertical: {} {ids}", if w.locked { "lock" } else { "unlock" });
    Ok(())
}

/// `vertical.flagAttention`: one mark per frame (its note updated), sorted by frame.
fn flag_attention(ctx: &Ctx, draft: &mut Draft, w: &FlagAttention) -> Result<(), Refusal> {
    chars_within("note", &w.note, MAX_NOTE)?;
    marks_guard(ctx, draft, w.plan)?;
    ctx.frame_in("frame", w.frame)?;
    let r = ctx.r;
    let shot = ctx.scene().shot_index(w.frame) + 1;
    draft.plan = Some(w.plan);
    let plan = draft.plan_mut(w.plan)?;
    let at = r.rate.tick_of(w.frame);
    let mark = AttentionMark { at, by: w.by.clone(), when: w.when, note: w.note.clone() };
    match plan.attention.iter_mut().find(|m| m.at == at) {
        Some(existing) if existing.note == w.note => {}
        Some(existing) => *existing = mark,
        None => {
            let index = plan.attention.partition_point(|m| m.at < at);
            plan.attention.insert(index, mark);
        }
    }
    if plan.attention.len() > MAX_MARKS {
        return Err(too_many("attention", plan.attention.len(), MAX_MARKS));
    }
    draft.label = format!("Vertical: flag shot {shot}");
    Ok(())
}

/// `vertical.clearAttention`: frames without a mark are not an error.
fn clear_attention(ctx: &Ctx, draft: &mut Draft, w: &ClearAttention) -> Result<(), Refusal> {
    check_len("frames", w.frames.len(), MAX_LIST)?;
    marks_guard(ctx, draft, w.plan)?;
    let rate = ctx.r.rate;
    draft.plan = Some(w.plan);
    let plan = draft.plan_mut(w.plan)?;
    plan.attention.retain(|m| !w.frames.contains(&rate.frame_at(m.at)));
    draft.label = "Vertical: clear attention".into();
    Ok(())
}

/// `vertical.update`: names, sizes (same aspect; positions below a larger output's `h_min` refused or, with `clamp`,
/// enlarged about their centres), filter, starting window, captions and fit background.
fn update(ctx: &Ctx, draft: &mut Draft, w: &UpdatePlan) -> Result<(), Refusal> {
    let before = guard(ctx, w.plan, w.expect.as_deref(), None)?;
    let r = ctx.r;
    if let Some(name) = &w.name {
        check_plan_name(name)?;
        if let Some(other) = draft.set.plans.iter().find(|p| p.id != w.plan && p.name == *name) {
            return Err(Refusal::new("plan_exists", format!("a plan is named {name:?} already"), json!({"id": other.id})));
        }
    }
    let output = match (w.width, w.height) {
        (None, None) => before.output,
        (width, height) => check_output(width, height)?,
    };
    if !same_aspect(output, before.output) {
        let detail = json!({"width": output.width, "height": output.height});
        return Err(Refusal::new("aspect_change", "a plan's size keeps its aspect; make another plan for another aspect", detail));
    }
    let heights = check_limits(&r, output)?;
    let small: Vec<&Position> = before.positions.iter().filter(|p| p.window.h < heights.min_height).collect();
    if !small.is_empty() && !w.clamp {
        let ids: Vec<u32> = small.iter().map(|p| p.id).collect();
        let detail = json!({"minHeight": heights.min_height, "maxHeight": heights.max_height, "positions": ids});
        return Err(Refusal::new("size_limit", "positions are smaller than the new output allows; pass clamp to enlarge them", detail));
    }
    let held: Vec<u32> = small.iter().filter(|p| p.locked).map(|p| p.id).collect();
    if !held.is_empty() {
        return Err(locked(held));
    }
    if let Some(captions) = &w.captions {
        check_captions(captions, output)?;
    }
    let starting = match w.starting_window {
        Some(Some(s)) => {
            check_coordinates("startingWindow", &s)?;
            let mut ignored = Vec::new();
            Some(Some(store_window(&r, output, s, w.clamp, (0, 0), &mut ignored)?))
        }
        other => other,
    };
    draft.plan = Some(w.plan);
    let mut clamped = Vec::new();
    let plan = draft.plan_mut(w.plan)?;
    if let Some(name) = &w.name {
        plan.name = name.clone();
    }
    plan.output = output;
    for p in plan.positions.iter_mut().filter(|p| p.window.h < heights.min_height) {
        let enlarged = clamp_window(Window { h: heights.min_height, ..p.window }, &r, output);
        clamped.push(Clamped { position: p.id, dx: enlarged.cx - p.window.cx, dy: enlarged.cy - p.window.cy, dh: enlarged.h - p.window.h });
        p.window = enlarged;
    }
    if let Some(filter) = w.filter {
        plan.filter = filter;
    }
    if let Some(background) = w.fit_background {
        plan.fit_background = background;
    }
    if let Some(captions) = &w.captions {
        plan.captions = captions.clone();
    }
    if let Some(starting) = starting {
        plan.starting_window = starting;
    }
    let name = plan.name.clone();
    draft.clamped = clamped;
    draft.label = format!("Vertical: update plan {name}");
    Ok(())
}

/// A plan's caption layout checked: 1 to 4 lines, sizes positive and finite, an area with a size inside the output.
fn check_captions(c: &VerticalCaptions, out: OutputSize) -> Result<(), Refusal> {
    if !(1..=4).contains(&c.max_lines) {
        return Err(Refusal::new("out_of_range", "subtitles take 1 to 4 lines", json!({"key": "maxLines", "value": c.max_lines})));
    }
    for (key, size) in [("size", c.size), ("minSize", c.min_size)] {
        if size.is_some_and(|s| !s.is_finite() || s <= 0.0) {
            return Err(bad_number(key));
        }
    }
    if let Some([x, y, w, h]) = c.area {
        let inside = u64::from(x) + u64::from(w) <= u64::from(out.width) && u64::from(y) + u64::from(h) <= u64::from(out.height);
        if w == 0 || h == 0 || !inside {
            return Err(Refusal::new("out_of_range", "the subtitle area lies inside the output", json!({"key": "area", "area": [x, y, w, h]})));
        }
    }
    Ok(())
}

/// Whether a position's change counts as a change (spec section 7.5): window, frame, layout, arrival, caption anchor,
/// label, tags, note, colour, subjects, focus, confidence or risk. Origin, `modifiedBy`, `reviewed`, flags and
/// `proposed` follow from a change.
fn framing_changed(a: &Position, b: &Position) -> bool {
    let placed = a.at != b.at || a.window != b.window || a.layout != b.layout || a.arrival != b.arrival;
    let described = a.caption_anchor != b.caption_anchor || a.label != b.label || a.tags != b.tags || a.note != b.note || a.color != b.color;
    placed || described || a.subjects != b.subjects || a.focus != b.focus || a.confidence != b.confidence || a.risk != b.risk
}

/// What a write did to one plan's positions, by id.
struct Diff {
    added: Vec<u32>,
    changed: BTreeSet<u32>,
    updated: Vec<u32>,
    removed: Vec<u32>,
}

/// The positions of a plan by id (the first of an id).
fn by_id(plan: &VerticalPlan) -> BTreeMap<u32, &Position> {
    let mut map = BTreeMap::new();
    for p in &plan.positions {
        map.entry(p.id).or_insert(p);
    }
    map
}

/// Positions added, changed (spec section 7.5), updated (any stored field) and removed, in plan order.
fn diff(before: &VerticalPlan, after: &VerticalPlan) -> Diff {
    let (old, new) = (by_id(before), by_id(after));
    let mut d = Diff { added: Vec::new(), changed: BTreeSet::new(), updated: Vec::new(), removed: Vec::new() };
    for p in &after.positions {
        match old.get(&p.id) {
            None => d.added.push(p.id),
            Some(b) => {
                if framing_changed(b, p) {
                    d.changed.insert(p.id);
                }
                if *b != p {
                    d.updated.push(p.id);
                }
            }
        }
    }
    d.removed = before.positions.iter().filter(|b| !new.contains_key(&b.id)).map(|b| b.id).collect();
    d
}

/// The frames whose window or layout differ between two versions of a plan, merged and half-open (spec section 7.2): it
/// compares `window_at` over every frame a changed position can affect.
pub(crate) fn frames_changed(r: &Raster, before: &VerticalPlan, after: &VerticalPlan) -> Vec<[i64; 2]> {
    let rate = r.rate;
    let end = r.end_frame;
    let (old, new) = (by_id(before), by_id(after));
    let moved = |a: &Position, b: &Position| a.at != b.at || a.window != b.window || a.layout != b.layout || a.arrival != b.arrival;
    let mut touched: Vec<i64> = Vec::new();
    for p in &after.positions {
        if old.get(&p.id).is_none_or(|b| moved(b, p)) {
            touched.push(rate.frame_at(p.at));
        }
    }
    for b in &before.positions {
        if new.get(&b.id).is_none_or(|p| p.at != b.at) {
            touched.push(rate.frame_at(b.at));
        }
    }
    let mut spans: Vec<[i64; 2]> = Vec::new();
    if before.positions.is_empty() && after.positions.is_empty() {
        spans.push([0, end]);
    }
    let (frames_before, frames_after) = (frames_of(before, rate), frames_of(after, rate));
    for t in touched {
        spans.push(influence(&frames_before, t, end));
        spans.push(influence(&frames_after, t, end));
    }
    let mut out: Vec<[i64; 2]> = Vec::new();
    for [a, b] in merge_ranges(spans) {
        for f in a.max(0)..b.min(end) {
            let tick = rate.tick_of(f);
            let (x, y) = (window_at(before, r, tick), window_at(after, r, tick));
            if x.window == y.window && x.layout == y.layout {
                continue;
            }
            match out.last_mut() {
                Some(run) if run[1] == f => run[1] = f + 1,
                _ => out.push([f, f + 1]),
            }
        }
    }
    out
}

/// The frames of a plan's positions, sorted.
fn frames_of(plan: &VerticalPlan, rate: FrameRate) -> Vec<i64> {
    let mut frames: Vec<i64> = plan.positions.iter().map(|p| rate.frame_at(p.at)).collect();
    frames.sort_unstable();
    frames
}

/// The frames whose window can depend on a position at frame `t` of a plan with positions on `frames` (or on one added
/// or removed there): from the position before it to the one after it.
fn influence(frames: &[i64], t: i64, end: i64) -> [i64; 2] {
    let before = frames.partition_point(|&f| f < t).checked_sub(1).and_then(|i| frames.get(i)).copied().unwrap_or(0);
    let after = frames.get(frames.partition_point(|&f| f <= t)).copied().unwrap_or(end);
    [before.min(t), after.max(t.saturating_add(1))]
}

/// The invariants a write's changed positions must hold (I3, I4) and every move of the plan (I5, I6, I9), with the
/// refusal that names the first broken one.
fn check_plan(scene: &Scene, plan: &VerticalPlan, changed: &BTreeSet<u32>) -> Result<(), Refusal> {
    let r = scene.r;
    let rate = r.rate;
    for p in plan.positions.iter().filter(|p| changed.contains(&p.id)) {
        check_window(&r, plan.output, &p.window, rate.frame_at(p.at))?;
    }
    let mut fit = Vec::new();
    for pair in plan.positions.windows(2) {
        let [departing, p] = pair else { continue };
        let Arrival::Move { move_frames, .. } = p.arrival else { continue };
        let (f, previous) = (rate.frame_at(p.at), rate.frame_at(departing.at));
        let s = f.saturating_sub(i64::from(move_frames));
        if s < previous {
            let detail = json!({"position": p.id, "earliestStart": previous, "maxMoveFrames": f - previous});
            return Err(Refusal::new("move_overlaps", "the move would start before the position it departs from", detail));
        }
        if departing.layout == Layout::Fit || p.layout == Layout::Fit {
            fit.push(p.id);
            continue;
        }
        let walls = &scene.wall_frames;
        if let Some(&wall) = walls.partition_point(|&c| c <= f).checked_sub(1).and_then(|i| walls.get(i)).filter(|&&c| c > s) {
            return Err(crosses(scene, p.id, f, wall));
        }
    }
    if !fit.is_empty() {
        return Err(Refusal::new("layout_move", "a move never arrives at or leaves a fit position", json!({"ids": fit})));
    }
    Ok(())
}

/// Turns the draft into the write's result: the people's-work and invariant checks, the change semantics of spec
/// section 7.5 for a person, the deltas, the changed frames, the issues and the label.
fn finish(ctx: &Ctx, mut draft: Draft) -> Result<Planned, Refusal> {
    let before_set = ctx.before;
    let empty = empty_set();
    let old = before_set.unwrap_or(&empty);
    let mut diffs: Vec<(u32, Diff)> = Vec::new();
    for plan in &mut draft.set.plans {
        let Some(before) = old.plans.iter().find(|p| p.id == plan.id) else { continue };
        let d = diff(before, plan);
        let old_ids = by_id(before);
        let was_locked = |id: &u32| old_ids.get(id).is_some_and(|b| b.locked);
        let held: Vec<u32> = d.changed.iter().chain(&d.removed).copied().filter(was_locked).collect();
        if !held.is_empty() {
            return Err(locked(held));
        }
        // spec section 5.10: a person's direct edit that changes exactly one position marks it reviewed; the positions it
        // changes only as a by-product of the one it names do not count
        let pid = plan.id;
        let named: Vec<u32> = d.changed.iter().chain(&d.added).copied().filter(|id| !draft.side.contains(&(pid, *id))).collect();
        let one = named.len() == 1;
        for p in &mut plan.positions {
            if d.changed.contains(&p.id) {
                p.modified_by = ctx.caller.writer;
                if !draft.flagged.contains(&(pid, p.id)) {
                    p.flags.clear();
                }
            }
            if draft.direct && one && ctx.caller.writer == Writer::Human && named.contains(&p.id) {
                p.reviewed = true;
            }
        }
        diffs.push((pid, diff(before, plan)));
    }
    let unchanged = match before_set {
        Some(set) => draft.set == *set,
        None => draft.set == empty,
    };
    if unchanged {
        let set = before_set.map(|s| Box::new(s.clone()));
        let (plan, missing, ids, issues) = (draft.plan, draft.missing, draft.ids, draft.issues);
        return Ok(Planned { set, changed: false, plan, missing, ids, issues, ..Planned::default() });
    }
    let scene = Scene::new(ctx.project, ctx.seq, Some(&draft.set));
    // the plans the write changed, each with whether it held every invariant before the write
    let mut written: Vec<(u32, Diff, bool)> = Vec::new();
    for (id, d) in diffs {
        let (Some(before), Some(after)) = (old.plans.iter().find(|p| p.id == id), draft.set.plans.iter().find(|p| p.id == id)) else { continue };
        if d.added.is_empty() && d.updated.is_empty() && d.removed.is_empty() && before.starting_window == after.starting_window {
            continue;
        }
        let count = after.positions.len();
        if count > MAX_POSITIONS && count > before.positions.len() {
            return Err(too_many("positions", count, MAX_POSITIONS));
        }
        // a strict write's guards found its plans valid; a lenient one may start from a plan that breaks an invariant
        let valid = !draft.lenient || plan_violations(&ctx.scene(), before).is_empty();
        if valid {
            let touched: BTreeSet<u32> = d.added.iter().chain(&d.changed).copied().collect();
            check_plan(&scene, after, &touched)?;
        }
        written.push((id, d, valid));
    }
    let added = added_violations(ctx, &scene, old);
    if !added.is_empty() {
        return Err(Refusal::new("invalid_plan", "the write would break an invariant", json!({"violations": added})));
    }
    let mut plans = Vec::new();
    let mut issues = draft.issues.clone();
    for (id, d, valid) in written {
        let (Some(before), Some(after)) = (old.plans.iter().find(|p| p.id == id), draft.set.plans.iter().find(|p| p.id == id)) else { continue };
        // spec section 2.8: a plan that still breaks an invariant is never evaluated; its issue is `invalid_plan`
        let violations = if valid { Vec::new() } else { plan_violations(&scene, after) };
        if !violations.is_empty() {
            let issue = Issue { code: "invalid_plan", level: Level::Error, plan: Some(id), frames: None, detail: json!({"violations": violations}) };
            if !issues.contains(&issue) {
                issues.push(issue);
            }
            plans.push(PlanDelta { plan: id, added: d.added, updated: d.updated, removed: d.removed, changed_frames: Vec::new() });
            continue;
        }
        let changed_frames = frames_changed(&ctx.r, before, after);
        if Some(id) == draft.plan
            && let Some([a, b]) = draft.strict
            && changed_frames.iter().any(|&[x, y]| x < a || y > b)
        {
            let detail = json!({"changedFrames": changed_frames, "range": [a, b]});
            return Err(Refusal::new("range_exceeded", "the write changes frames outside its range", detail));
        }
        let rate = ctx.r.rate;
        let mut ranges = changed_frames.clone();
        for p in after.positions.iter().filter(|p| d.added.contains(&p.id) || d.updated.contains(&p.id)) {
            let f = rate.frame_at(p.at);
            ranges.push([f, f.saturating_add(1)]);
        }
        let facts = Facts::new(&scene, after);
        for issue in plan_issues_near(&scene, &facts, Some(ranges.as_slice())).into_iter().filter(|i| touches(i, &ranges)) {
            if !issues.contains(&issue) {
                issues.push(issue);
            }
        }
        if !d.added.is_empty() || !d.updated.is_empty() || !d.removed.is_empty() || !changed_frames.is_empty() {
            plans.push(PlanDelta { plan: id, added: d.added, updated: d.updated, removed: d.removed, changed_frames });
        }
    }
    let named = plans.iter().find(|p| Some(p.plan) == draft.plan).cloned().unwrap_or_default();
    let label = if draft.label.is_empty() { "Vertical: edit".to_string() } else { draft.label };
    Ok(Planned {
        set: Some(Box::new(draft.set)),
        changed: true,
        plan: draft.plan,
        added: named.added,
        updated: named.updated,
        removed: named.removed,
        missing: draft.missing,
        ids: draft.ids,
        changed_frames: named.changed_frames,
        plans,
        clamped: draft.clamped,
        issues,
        changes: draft.changes,
        label,
    })
}

/// The violations of the written data that the data before the write did not have (spec section 2.5: a write that breaks
/// an invariant is refused and changes nothing). It checks the set's own data when the write changed it, and every plan
/// the write changed, or every plan when the set's data changed (its walls and parts bind every plan). The specific
/// refusals come first (`check_plan`, `too_many`, the write's own); this is the full list of I1-I21 and I23.
fn added_violations(ctx: &Ctx, after: &Scene, old: &VerticalSet) -> Vec<Violation> {
    let Some(new) = after.set else { return Vec::new() };
    let shared = shared_changed(old, new);
    let mut found = if shared { set_violations(after, new) } else { Vec::new() };
    for plan in &new.plans {
        if shared || old.plans.iter().find(|p| p.id == plan.id) != Some(plan) {
            found.extend(plan_only_violations(after, plan));
        }
    }
    if found.is_empty() {
        return found;
    }
    let before = ctx.scene();
    let mut known = set_violations(&before, old);
    for plan in &old.plans {
        known.extend(plan_only_violations(&before, plan));
    }
    found.retain(|v| !known.contains(v));
    found
}

/// Whether a write changed the set's own data: its walls, parts, regions, annotations, box tracks or active areas, or the
/// plans it holds and their names (I11).
fn shared_changed(a: &VerticalSet, b: &VerticalSet) -> bool {
    let names = |s: &VerticalSet| s.plans.iter().map(|p| (p.id, p.name.clone())).collect::<Vec<_>>();
    let data = a.walls != b.walls || a.parts != b.parts || a.regions != b.regions || a.annotations != b.annotations;
    data || a.box_tracks != b.box_tracks || a.active_area != b.active_area || names(a) != names(b)
}
