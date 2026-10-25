//! Vertical Edit: the framing of a 16:9 sequence for a vertical (or any other) output raster.
//!
//! A [`VerticalSet`] lives on [`Sequence::vertical`](crate::Sequence::vertical). Each of its plans holds positions: a window
//! `(cx, cy, h)` at an exact frame, reached by a cut or by a move along a curve. One function, [`window_at`], gives the
//! exact window at every tick, and every preview, read and export uses it. The 16:9 edit is never changed.
//!
//! Everything here is pure data and arithmetic (no session, UI or render state) and builds for wasm32. The rules are those
//! of the Vertical Edit specification v1.1 (final), sections 2 and 3.
//!
//! Milestone M2 adds the model behind every engine command and read: the one planner ([`plan_write`]), the effective
//! [`walls`], parts and their timecode, protected regions, annotations, the active area, the derived values
//! ([`derive_plan`]), the motion measures ([`measure`]), `vertical.fit` ([`fit::fit`]), the part edges ([`edge_facts`]),
//! the output and range digests, and validation with repair ([`validate`]). Functions that need the effective walls take
//! the [`Project`](crate::Project) too: it says which clips show picture.
//!
//! Its second stage adds the edit hook ([`follow_picture`]): the framing and every frame-based datum follow each edit of
//! the 16:9 picture exactly, or are flagged; and `vertical.remap` ([`plan_remap`]), the same machinery with a caller's map.

pub mod annotations;
pub mod area;
pub mod canon;
pub mod curve;
pub mod derive;
pub mod digest;
pub mod edges;
pub mod eval;
pub mod fit;
pub mod follow;
pub mod measure;
pub mod output_digest;
pub mod parts;
pub mod path;
pub mod plan;
pub mod regions;
pub mod remap;
pub mod smooth;
pub mod split;
pub mod types;
pub mod validate;
pub mod walls;

pub use annotations::{MAX_ANNOTATIONS, valid_kind, valid_track_id};
pub use area::{MAX_AREAS, active_issues, active_rect, clamp_to_area, largest_in_area};
pub use canon::{CentreBounds, ceil_g, centre_bounds, clamp_window, floor_g, floor_micro, h_from_zoom, on_grid, q, quantize_window, zoom_percent};
pub use curve::{DEFAULT_CURVE, MAX_CURVE_SLOPE, SlopePeak, check_curve, ease};
pub use derive::{
    Derived, Event, FrameRow, GoTo, LabelSource, LibraryEntry, PartState, PositionInfo, Shot, ShotStatus, Tag, derive_plan, events, frame_rows, from_label,
    go_to, governing, issues, library, raster, shots,
};
pub use digest::{data_digest, effective_walls_digest, parts_digest, plan_digest, range_digest, regions_digest, walls_digest};
pub use edges::{
    CueDistances, CueStraddle, Dissolve, EdgeFacts, EdgeSide, EdgeThresholds, NearestAnnotation, PartEdges, PartGap, WallDistance, edge_facts, part_edges,
};
pub use eval::{MoveSpan, centred_largest, limits, move_span, window_at};
pub use fit::{EyeTrace, FitRequest, FitResult, LimitedBy};
pub use follow::{
    FollowReport, PictureDigests, PlanFollow, SequenceDigests, SequenceFollow, follow_bound, follow_picture, follow_picture_with, framing_digest, hook_bound,
    layout_digest, picture_digests,
};
pub use measure::{
    BreakKind, BreakMeasure, CaptionBox, EyeTraceMeasure, JumpMeasure, JumpSimilar, JunctionMeasure, LabelSpread, MeasureOpts, Measures, MoveMeasure,
    PartMeasure, RegionBox, RegionInput, RegionMeasure, Series, ShotMeasure, Thresholds, Totals, measure,
};
pub use output_digest::output_digest;
pub use parts::{PartStatus, format_part_timecode, parse_part_timecode, part_status, unreviewed_shots};
pub use path::path;
pub use plan::{
    AnnotationAdd, AnnotationItem, AnnotationReplace, AnnotationUpdate, ApplyMeasurement, ApprovePart, AreaSet, ArrivalKind, CandidateAt, CandidateSet,
    Clamped, ClearAttention, CreatePlan, DeletePlan, DeletePositions, EdgeMove, FitMove, FlagAttention, MAX_LIST, MAX_REPLACE_ITEMS, MarkChecked, MarkerFilter,
    MeasuredFrame, Measurement, OnConflict, PERSON, PartAdd, PartMarkers, PartUpdate, PlanDelta, Planned, PositionFields, Preset, RangeGuard, RegionAdd,
    RegionUpdate, Repair, ReplaceItem, ReplaceRange, RetimePosition, SeedMode, SeedShots, SetActiveArea, SetAnnotations, SetArrival, SetLock, SetParts,
    SetPosition, SetRegions, SetReview, SetWalls, SmoothThrough, SwapWindows, UpdatePlan, WallAdd, WallMove, Write, frame_of_time, plan_write,
};
pub use regions::{RegionRun, RegionState, region_issues, region_runs, region_state, visible_fraction};
pub use remap::{MAX_MAP, RemapEntry, plan_remap, remap_digest};
pub use smooth::{screen_speed, smooth_through};
pub use split::{Split, insert_split, move_label, split_move};
pub use types::*;
pub use validate::{Violation, validate};
pub use walls::{Wall, WallSource, edit_density, geometry_differs, is_shot_change, is_through_edit, shown_frame, unknown_ranges, walls};

#[cfg(test)]
mod tests;
