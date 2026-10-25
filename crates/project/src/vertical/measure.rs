//! Motion measures (spec section 3.9) and the threshold findings of `vertical.measure` (spec sections 3.9, 4.3, 6.4 and
//! 7.6). Everything is computed from data: the windows `window_at` gives, the walls, the cues of the plan's caption tracks,
//! the focus points, the regions and the caption boxes the caller passes. A finding exists only for a threshold the caller
//! passes; the raw numbers are always there. Sums and maxima are taken sequentially in frame order, so the bits never
//! depend on how many threads a caller runs (C11).
//!
//! For consecutive frames `f - 1`, `f` with windows `(x, y, k)` (`k = h / OH`), the content at output point `o` moves on
//! screen by `δ_f(o) = o · (1 - k_f / k_{f-1}) - (corner_f - corner_{f-1}) / k_{f-1}` (the form of
//! [`screen_speed`](super::smooth::screen_speed)); the speed arriving at `f` is the largest length of `δ_f` over the four
//! output corners. A frame a cut arrives at, and a `fit` frame, carry no speed: a break is not motion.

use filmcraft_time::Tick;
use serde::Serialize;
use serde_json::json;

use super::area::active_rect;
use super::curve::{bezier_max_slope, ease};
use super::derive::{Facts, Scene, Shot, ShotStatus, Tag, arriving_speed, governing, issue, shots_of, upscale_of};
use super::eval::{MoveSpan, window_at};
use super::path::path;
use super::plan::{invalid_plan, not_found_plan};
use super::regions::{RegionRun, RegionState, mode_warning, runs_of, visible_in};
use super::types::{Curve, Issue, Layout, Level, OutputSize, Phase, Position, Raster, Rect, Refusal, RegionMode, Sample, VE_MAX_FRAMES, VerticalPlan, Window};
use super::validate::plan_violations;
use super::walls::shown_frame;
use crate::{Project, Sequence};

/// No corner moves.
const STILL: [[f64; 2]; 4] = [[0.0; 2]; 4];

/// `jumpSimilar {maxShift, maxScale}`: a JUMP is `jump_similar` only when its shift and its scale are both below these.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JumpSimilar {
    /// Output widths.
    pub max_shift: f64,
    /// A ratio of at least 1.
    pub max_scale: f64,
}

/// The thresholds of `vertical.measure` (spec section 7.4). Each one the caller passes turns its finding on.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Thresholds {
    /// `upscale_over`.
    pub max_upscale: Option<f64>,
    /// `speed_over` (output px per frame); also the sustained-speed measure.
    pub max_speed: Option<f64>,
    /// `accel_over` (output px per frame squared).
    pub max_accel: Option<f64>,
    /// `short_hold` (frames).
    pub min_hold_frames: Option<i64>,
    /// `ping_pong`: a reversal of the centre's motion within this many frames.
    pub ping_pong_frames: Option<i64>,
    /// `near_wall_change` (W7).
    pub near_wall_frames: Option<i64>,
    /// `speed_step_over` (output px per frame).
    pub max_speed_step: Option<f64>,
    /// `jump_similar`.
    pub jump_similar: Option<JumpSimilar>,
    /// `small_move` (output px).
    pub min_move_distance: Option<f64>,
    /// `early_move` (frames from a wall to the shot's first moving frame).
    pub min_hold_after_wall: Option<i64>,
    /// `long_hold` (frames).
    pub max_hold_frames: Option<i64>,
    /// `cue_straddles_cut`: a cue on screen on both sides of a break with a side of 1 to this many frames.
    pub straddle_frames: Option<i64>,
    /// `eye_trace_jump` (output widths).
    pub max_eye_trace_jump: Option<f64>,
}

/// One box of a caller's region, at a frame, in sequence px.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct RegionBox {
    pub frame: i64,
    pub rect: [f64; 4],
}

/// A caller's region (sequence space): present from its first box's frame through its last; between boxes it holds, or
/// interpolates linearly within a shot when `interpolate`; never across a wall (the earlier box holds up to the wall, and
/// the region is absent after it until the next box).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RegionInput {
    pub id: String,
    pub label: String,
    pub boxes: Vec<RegionBox>,
    pub interpolate: bool,
}

/// A laid-out caption box of the vertical output over a frame range, in output px (`{source: "captions"}`, lane L3b's).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CaptionBox {
    pub cue: u64,
    pub from_frame: i64,
    pub to_frame: i64,
    pub rect: [f64; 4],
}

/// The options of `vertical.measure` the model takes; the engine resolves `{source}` regions and the safe-zone profile.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeasureOpts {
    /// Default the plan's first frame, 0.
    pub from_frame: Option<i64>,
    /// Default the sequence's end.
    pub to_frame: Option<i64>,
    pub thresholds: Thresholds,
    pub regions: Vec<RegionInput>,
    /// `{source: "protected"}`: the set's protected regions, each checked against its mode.
    pub protected: bool,
    pub captions: Vec<CaptionBox>,
    /// The safe zone in output px; a box reaching outside it on a frame is a safe-zone hit.
    pub safe_zone: Option<[f64; 4]>,
    /// Per-frame arrays.
    pub series: bool,
    /// `baseline: "centreCrop"`: the same numbers for a static, centred, largest window beside the plan's.
    pub baseline: bool,
}

/// One move (spec section 3.9).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveMeasure {
    /// The position it arrives at.
    pub position: u32,
    /// The position it departs from.
    pub from: u32,
    /// `s`, its start frame, which shows the departing window.
    pub start: i64,
    /// `f_j`, its arrival frame.
    pub end: i64,
    pub frames: u32,
    /// The largest corner displacement of the whole move, output px (`D_out` of spec section 3.8).
    pub distance: f64,
    /// `distance / frames`.
    pub mean_speed: f64,
    /// The largest arriving speed over its frames, output px per frame.
    pub peak_speed: f64,
    /// The largest corner acceleration over its frames plus one frame of rest on each side, output px per frame squared.
    pub peak_accel: f64,
    /// The acceleration leaving rest (arriving at `s + 1`) and coming to rest (arriving at `f_j + 1`).
    pub start_accel: f64,
    pub end_accel: f64,
    /// The largest third difference, the same way.
    pub peak_jerk: f64,
    /// The largest `|ln(h_f / h_{f-1})|`.
    pub peak_zoom_rate: f64,
}

/// A junction: a move ends exactly where the next starts (`s_{k+1} = f_k`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JunctionMeasure {
    pub frame: i64,
    /// The position whose move ends there, and the one whose move starts there.
    pub into: u32,
    pub out_of: u32,
    /// `|v_{f+1} - v_f|`.
    pub speed_step: f64,
    /// The angle in degrees between the centre's displacement arriving at the junction and leaving it; None when either is
    /// zero.
    pub turn: Option<f64>,
}

/// A `JUMP` from window `b` (frame `c - 1`) to window `a` (frame `c`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JumpMeasure {
    pub position: u32,
    pub frame: i64,
    /// `‖(c_b - c_a) / k_a‖ / OW`: where `b`'s centre lands relative to `a`'s centre, in output widths.
    pub shift: f64,
    /// `max(h_b, h_a) / min(h_b, h_a)`.
    pub scale: f64,
    /// Intersection over union of the two windows (data only).
    pub iou: f64,
}

/// A window break's kind: a cut on a wall or an in-shot jump.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BreakKind {
    Cut,
    Jump,
}

/// The focus across a break: its output positions on frames `c - 1` and `c` and their distance in output widths.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EyeTraceMeasure {
    pub before: [f64; 2],
    pub after: [f64; 2],
    pub distance: f64,
}

/// One window break: a frame where the window changes by a cut (spec section 2.1).
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BreakMeasure {
    pub position: u32,
    pub frame: i64,
    pub kind: BreakKind,
    /// When the governing positions of both frames have a focus point; else the break is not measured.
    pub eye_trace: Option<EyeTraceMeasure>,
    /// The direction, in degrees, of the centre's motion arriving at `c - 1` and at `c + 1`, when the window moves on both
    /// sides.
    pub direction: Option<[f64; 2]>,
    /// The signed frame distance from the break to the nearest cue start of the plan's caption tracks, and to the nearest
    /// cue end (a cue's end frame is the first frame after it).
    pub cue_start: Option<i64>,
    pub cue_end: Option<i64>,
}

/// One shot.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShotMeasure {
    pub number: u32,
    pub from: i64,
    pub to: i64,
    pub status: ShotStatus,
    pub positions: u32,
    pub moves: u32,
    pub jumps: u32,
    /// Frames from the wall until the window stops moving: the end of the first chain of moves in the shot (0 without).
    pub settle: i64,
    /// The share of the shot's frames at zero speed.
    pub still_share: f64,
    /// Frames from the wall to the shot's first moving frame; None when the shot never moves.
    pub hold_after_wall: Option<i64>,
    /// The shortest and the longest hold between consecutive JUMPs of the shot.
    pub jump_rhythm: Option<[i64; 2]>,
    /// With `pingPongFrames`: the frames where the centre's motion reverses, counted over move frames only.
    pub reversals: Vec<i64>,
}

/// The spread of one label's framings: the population standard deviations of `cx`, `cy` and `h` (sequence px), and of
/// the focus's output position (output px) over the positions that have one.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelSpread {
    pub label: String,
    pub positions: u32,
    pub cx: f64,
    pub cy: f64,
    pub h: f64,
    pub focus: Option<[f64; 2]>,
}

/// One part.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartMeasure {
    pub part: u32,
    /// The share of the part's frames in `fit` layout.
    pub fit_share: f64,
    /// The number of fit-fill changes inside the part.
    pub fit_changes: u32,
    /// Changes of the effective caption anchor between consecutive cues of the part, per minute of the part (spec
    /// section 6.4: a cue takes the anchor of the position governing its first frame, else the plan's).
    pub anchor_flips_per_minute: f64,
}

/// One region (a caller's, or a protected one).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionMeasure {
    /// The caller's id; empty for a protected region.
    pub id: String,
    /// The protected region's id.
    pub region: Option<u32>,
    pub label: String,
    /// The protected region's mode.
    pub mode: Option<RegionMode>,
    pub frames_in: u32,
    pub frames_sliced: u32,
    pub frames_out: u32,
    pub runs: Vec<RegionRun>,
    /// The frame with the smallest visible fraction (the largest for `keepOut`), the earliest on a tie.
    pub worst_frame: Option<i64>,
    /// At the worst frame, output px from the box to the window's left, top, right and bottom edges (negative where the
    /// edge cuts the box).
    pub margins: Option<[f64; 4]>,
    pub safe_zone_hits: u32,
}

/// Totals over the measured frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    pub frames: i64,
    /// The smallest distance from a window to the frame's edges, sequence px.
    pub min_margin: f64,
    pub max_upscale: f64,
    pub carried_shots: u32,
    /// Always 0 in a valid plan: asserted, not measured.
    pub motion_across_walls: f64,
    /// With `maxSpeed`: the longest run of consecutive frames whose speed exceeds it.
    pub sustained_speed: Option<i64>,
}

/// Per-frame arrays, in frame order.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Series {
    pub frames: Vec<i64>,
    pub speed: Vec<f64>,
    pub accel: Vec<f64>,
    pub zoom_rate: Vec<f64>,
    pub upscale: Vec<f64>,
    /// Output pixels showing the bars outside the active area.
    pub bar_pixels: Vec<f64>,
}

/// What `vertical.measure` returns for one plan.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Measures {
    pub plan: u32,
    pub from_frame: i64,
    pub to_frame: i64,
    pub moves: Vec<MoveMeasure>,
    pub junctions: Vec<JunctionMeasure>,
    pub jumps: Vec<JumpMeasure>,
    pub breaks: Vec<BreakMeasure>,
    pub shots: Vec<ShotMeasure>,
    pub labels: Vec<LabelSpread>,
    pub parts: Vec<PartMeasure>,
    pub regions: Vec<RegionMeasure>,
    pub totals: Totals,
    /// The threshold findings, only for thresholds passed, plus the region warnings of the regions measured.
    pub findings: Vec<Issue>,
    pub series: Option<Series>,
    /// The centre-crop baseline over the same frames.
    pub baseline: Option<Box<Measures>>,
}

/// The measures of plan `plan` of `seq` over the frames of `opts`. `not_found {plan}` for a plan that does not exist,
/// `invalid_plan {violations}` for a plan that breaks an invariant, `out_of_range` for a range outside the sequence or
/// longer than `VE_MAX_FRAMES`.
pub fn measure(project: &Project, seq: &Sequence, plan: u32, opts: &MeasureOpts) -> Result<Measures, Refusal> {
    let scene = Scene::of(project, seq);
    let p = scene.plan(plan).ok_or_else(|| not_found_plan(plan))?;
    let violations = plan_violations(&scene, p);
    if !violations.is_empty() {
        return Err(invalid_plan(violations));
    }
    let end = scene.r.end_frame;
    let (from, to) = (opts.from_frame.unwrap_or(0), opts.to_frame.unwrap_or(end));
    let long = to.saturating_sub(from) > i64::from(VE_MAX_FRAMES);
    if from < 0 || from > to || to > end || long {
        let detail = json!({"key": "toFrame", "fromFrame": from, "toFrame": to, "end": end, "maximum": VE_MAX_FRAMES});
        return Err(Refusal::new("out_of_range", "the range lies inside the sequence, at most 1 000 000 frames long", detail));
    }
    let mut m = measure_in(&scene, p, from, to, opts);
    if opts.baseline {
        let centre = VerticalPlan { positions: Vec::new(), starting_window: None, ..p.clone() };
        let plain = MeasureOpts { baseline: false, ..opts.clone() };
        m.baseline = Some(Box::new(measure_in(&scene, &centre, from, to, &plain)));
    }
    Ok(m)
}

/// The corner displacements `δ(o)` of the window change from `prev` to `cur` (spec section 3.9), in the operation order
/// of [`screen_speed`](super::smooth::screen_speed). None for a `prev` with no usable height (bad data).
pub(crate) fn corner_deltas(prev: &Window, cur: &Window, out: OutputSize) -> Option<[[f64; 2]; 4]> {
    if !prev.h.is_finite() || prev.h <= 0.0 {
        return None;
    }
    let (ow, oh) = (f64::from(out.width), f64::from(out.height));
    let (dcx, dcy, dh) = (cur.cx - prev.cx, cur.cy - prev.cy, cur.h - prev.h);
    let gx = (dcx - dh * ow / (2.0 * oh)) * oh / prev.h;
    let gy = (dcy - dh / 2.0) * oh / prev.h;
    let corner = |ox: f64, oy: f64| [-(ox * dh / prev.h) - gx, -(oy * dh / prev.h) - gy];
    Some([corner(0.0, 0.0), corner(ow, 0.0), corner(0.0, oh), corner(ow, oh)])
}

/// The largest length of the corner displacements: the speed.
pub(crate) fn speed_of(d: &[[f64; 2]; 4]) -> f64 {
    d.iter().map(|[x, y]| (x * x + y * y).sqrt()).fold(0.0, f64::max)
}

/// The largest length of the corner differences of two displacements: the acceleration between them.
fn accel_of(d: &[[f64; 2]; 4], before: &[[f64; 2]; 4]) -> f64 {
    d.iter().zip(before).map(|([x, y], [a, b])| ((x - a) * (x - a) + (y - b) * (y - b)).sqrt()).fold(0.0, f64::max)
}

/// The largest length of the corners' third differences of position: `δ_k - 2 δ_{k-1} + δ_{k-2}`.
fn jerk_of(d: &[[f64; 2]; 4], d1: &[[f64; 2]; 4], d2: &[[f64; 2]; 4]) -> f64 {
    let mut peak = 0.0_f64;
    for ((c, b), a) in d.iter().zip(d1).zip(d2) {
        let (x, y) = (c[0] - 2.0 * b[0] + a[0], c[1] - 2.0 * b[1] + a[1]);
        peak = peak.max((x * x + y * y).sqrt());
    }
    peak
}

/// `|ln(h / h_before)|`, with the pure-Rust `libm` so every target gives the same bits; 0 for unusable heights.
fn zoom_rate(before: f64, h: f64) -> f64 {
    let rate = libm::log(h / before).abs();
    if rate.is_finite() { rate } else { 0.0 }
}

/// What a move's frames measure (spec section 3.9), over its frames plus one frame of rest on each side.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct MoveNumbers {
    pub peak_speed: f64,
    pub peak_accel: f64,
    pub start_accel: f64,
    pub end_accel: f64,
    pub peak_jerk: f64,
    pub peak_zoom_rate: f64,
    /// How many of its frames arrive with a speed above 0, and the first and last of them (as offsets from `s`).
    pub moving_frames: u32,
    pub first_moving: Option<u32>,
    pub last_moving: Option<u32>,
}

/// The numbers of a move from the windows of its frames `s ..= f_j`, in frame order.
pub(crate) fn move_numbers(windows: impl Iterator<Item = Window>, out: OutputSize) -> MoveNumbers {
    let mut n = MoveNumbers::default();
    let (mut d1, mut d2) = (STILL, STILL);
    let mut prev: Option<Window> = None;
    let mut k: u32 = 0;
    for w in windows {
        if let Some(p) = prev {
            k = k.saturating_add(1);
            let d = corner_deltas(&p, &w, out).unwrap_or(STILL);
            let speed = speed_of(&d);
            n.peak_speed = n.peak_speed.max(speed);
            if speed > 0.0 {
                n.moving_frames = n.moving_frames.saturating_add(1);
                n.first_moving = n.first_moving.or(Some(k));
                n.last_moving = Some(k);
            }
            let accel = accel_of(&d, &d1);
            if k == 1 {
                n.start_accel = accel;
            }
            n.peak_accel = n.peak_accel.max(accel);
            n.peak_jerk = n.peak_jerk.max(jerk_of(&d, &d1, &d2));
            n.peak_zoom_rate = n.peak_zoom_rate.max(zoom_rate(p.h, w.h));
            (d2, d1) = (d1, d);
        }
        prev = Some(w);
    }
    n.end_accel = accel_of(&STILL, &d1);
    n.peak_accel = n.peak_accel.max(n.end_accel);
    n.peak_jerk = n.peak_jerk.max(jerk_of(&STILL, &d1, &d2)).max(jerk_of(&STILL, &STILL, &d1));
    n
}

/// The numbers of the move `span` of `plan`, from `window_at` at each of its frames.
pub(crate) fn move_numbers_of(plan: &VerticalPlan, r: &Raster, span: &MoveSpan) -> MoveNumbers {
    let frames = span.start_frame..=span.end_frame;
    move_numbers(frames.map(|f| window_at(plan, r, r.rate.tick_of(f)).window), plan.output)
}

/// The window on frame `k` of a whole (unsplit) move of `d` frames from `a` to `b` along `curve`: exactly what `window_at`
/// gives there (`k = 0` is `a`, `k = d` is `b`, else the path at `E(k / d)`).
pub(crate) fn whole_move_window(a: &Window, b: &Window, d: u32, curve: &Curve, k: u32) -> Window {
    if k == 0 {
        return *a;
    }
    if k >= d {
        return *b;
    }
    path(a, b, ease(curve, f64::from(k) / f64::from(d)))
}

/// The numbers of a whole move of `d` frames from `a` to `b` along `curve`.
pub(crate) fn whole_move_numbers(a: &Window, b: &Window, d: u32, curve: &Curve, out: OutputSize) -> MoveNumbers {
    move_numbers((0..=d).map(|k| whole_move_window(a, b, d, curve, k)), out)
}

/// Where `curve` is fastest, as a progress `u`: the frames around it are probed first when a move is fitted to a speed.
pub(crate) fn fastest_point(curve: &Curve) -> f64 {
    match *curve {
        Curve::Linear | Curve::EaseInOut => 0.5,
        Curve::EaseIn => 2.0 / 3.0,
        Curve::EaseOut => 1.0 / 3.0,
        Curve::Speeds { start, end } => {
            let (linear, quadratic) = (2.0 * (3.0 - end - 2.0 * start), 3.0 * (start + end - 2.0));
            let vertex = if quadratic < 0.0 { -linear / (2.0 * quadratic) } else { -1.0 };
            if vertex > 0.0 && vertex < 1.0 {
                vertex
            } else if start >= end {
                0.0
            } else {
                1.0
            }
        }
        Curve::Bezier { x1, y1, x2, y2 } => bezier_max_slope(x1, y1, x2, y2).at,
        Curve::Ramp { r_in, r_out } => 0.5 * (r_in + 1.0 - r_out),
    }
}

/// The verdict on one candidate length of `vertical.fitMove`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fits {
    Holds,
    Fails,
    /// The search spent its budget of frame evaluations.
    Spent,
}

/// Whether the whole move of `d` frames from `a` to `b` along `curve` keeps every arriving speed at or below `max_speed`
/// and, when given, every acceleration (over its frames plus one frame of rest on each side) at or below `max_accel`:
/// exactly the numbers [`whole_move_numbers`] measures. The frames around the curve's fastest point are tried first,
/// then outward, so a length that fails usually fails on its first frames; `budget` counts the frames evaluated.
pub(crate) fn move_fits(a: &Window, b: &Window, d: u32, curve: &Curve, out: OutputSize, limits: (f64, Option<f64>), budget: &mut u64) -> Fits {
    let (max_speed, max_accel) = limits;
    // the frame arriving nearest the fastest point (a cast saturates, and NaN gives 0, which the clamp lifts to 1)
    let peak = ((fastest_point(curve) * f64::from(d)).round() as u32).clamp(1, d.max(1));
    let window = |k: u32| whole_move_window(a, b, d, curve, k);
    let delta = |k: u32| corner_deltas(&window(k.saturating_sub(1)), &window(k), out).unwrap_or(STILL);
    let mut step: u32 = 0;
    loop {
        let (up, down) = (peak.saturating_add(step), peak.checked_sub(step));
        let up_in = up <= d;
        let down_in = down.is_some_and(|k| k >= 1) && step > 0;
        if !up_in && !down_in {
            break;
        }
        for k in [up_in.then_some(up), down.filter(|_| down_in)].into_iter().flatten() {
            if *budget == 0 {
                return Fits::Spent;
            }
            *budget -= 1;
            if speed_of(&delta(k)) > max_speed {
                return Fits::Fails;
            }
        }
        step = step.saturating_add(1);
    }
    let Some(max_accel) = max_accel else { return Fits::Holds };
    let mut before = STILL;
    for k in 1..=d.saturating_add(1) {
        if *budget == 0 {
            return Fits::Spent;
        }
        *budget -= 1;
        let d_k = if k > d { STILL } else { delta(k) };
        if accel_of(&d_k, &before) > max_accel {
            return Fits::Fails;
        }
        before = d_k;
    }
    Fits::Holds
}

/// The output point of sequence point `p` on a frame showing `s` (spec section 2.3): through the window in `fill`, through
/// the whole frame at the output width, centred vertically, in `fit`.
pub(crate) fn output_point(s: &Sample, r: &Raster, out: OutputSize, p: [f64; 2]) -> [f64; 2] {
    let (ow, oh) = (f64::from(out.width), f64::from(out.height));
    match s.layout {
        Layout::Fill => {
            let rect = s.window.rect(out);
            let k = s.window.h / oh;
            [(p[0] - rect.x) / k, (p[1] - rect.y) / k]
        }
        Layout::Fit => {
            let (w, h) = (f64::from(r.width.max(1)), f64::from(r.height));
            let k = w / ow;
            let y0 = (oh - ow * h / w) / 2.0;
            [p[0] / k, p[1] / k + y0]
        }
    }
}

/// A sequence rect `[x, y, w, h]` mapped to output px on a frame showing `s`.
pub(crate) fn output_rect(s: &Sample, r: &Raster, out: OutputSize, rect: [f64; 4]) -> [f64; 4] {
    let [x0, y0] = output_point(s, r, out, [rect[0], rect[1]]);
    let [x1, y1] = output_point(s, r, out, [rect[0] + rect[2], rect[1] + rect[3]]);
    [x0, y0, x1 - x0, y1 - y0]
}

/// The population standard deviation of `xs`, summed sequentially: `sqrt(Σ (x - mean)² / n)`.
fn deviation(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    let n = xs.len() as f64;
    let mean = xs.iter().fold(0.0, |acc, x| acc + x) / n;
    let acc = xs.iter().fold(0.0, |acc, x| acc + (x - mean) * (x - mean));
    (acc / n).sqrt()
}

/// The direction of a displacement in degrees, `atan2(dy, dx)`; None for no displacement.
fn direction(dx: f64, dy: f64) -> Option<f64> {
    (dx != 0.0 || dy != 0.0).then(|| libm::atan2(dy, dx).to_degrees())
}

/// The angle between two displacements in degrees, `[0, 180]`; None when either is zero.
fn turn(a: [f64; 2], b: [f64; 2]) -> Option<f64> {
    let zero = |v: [f64; 2]| v[0] == 0.0 && v[1] == 0.0;
    if zero(a) || zero(b) {
        return None;
    }
    let cross = a[0] * b[1] - a[1] * b[0];
    let dot = a[0] * b[0] + a[1] * b[1];
    Some(libm::atan2(cross.abs(), dot).to_degrees())
}

/// One analysed move: its span, its numbers, its distance and the centre's whole displacement.
struct Trace {
    k: usize,
    span: MoveSpan,
    numbers: MoveNumbers,
    distance: f64,
    shift: [f64; 2],
}

/// The cue frames of the plan's caption tracks (spec section 6.4: its track, enabled or not, else every enabled track):
/// `(first frame, end frame, cue id, track id)`, sorted. A cue's end frame is the first frame after it.
pub(crate) fn cue_frames(scene: &Scene, plan: Option<&VerticalPlan>) -> Vec<(i64, i64, u64, u64)> {
    let rate = scene.r.rate;
    let pinned = plan.and_then(|p| p.captions.track);
    let tracks = scene.seq.caption_tracks.iter().filter(|t| match pinned {
        Some(id) => t.id.0 == u64::from(id),
        None => t.enabled,
    });
    let mut cues: Vec<(i64, i64, u64, u64)> = Vec::new();
    for t in tracks {
        for c in &t.captions {
            let start = shown_frame(rate, c.start);
            let end = shown_frame(rate, Tick(c.start.0.saturating_add(c.duration.0)));
            cues.push((start, end, c.id.0, t.id.0));
        }
    }
    cues.sort_unstable();
    cues
}

/// The signed distance from `frame` to the nearest of `frames` (the earlier one on a tie).
fn nearest(frames: impl Iterator<Item = i64>, frame: i64) -> Option<i64> {
    let mut best: Option<i64> = None;
    for f in frames {
        let d = f.saturating_sub(frame);
        best = match best {
            Some(b) if b.unsigned_abs() < d.unsigned_abs() || (b.unsigned_abs() == d.unsigned_abs() && b <= d) => Some(b),
            _ => Some(d),
        };
    }
    best
}

/// The measures of a plan in a scene over `[from, to)` (the range already checked).
fn measure_in(scene: &Scene, plan: &VerticalPlan, from: i64, to: i64, opts: &MeasureOpts) -> Measures {
    let th = opts.thresholds;
    let facts = Facts::new(scene, plan);
    let mut m = Measures { plan: plan.id, from_frame: from, to_frame: to, ..Measures::default() };
    let shots = shots_of(scene, &facts);
    let shown: Vec<_> = shots.iter().filter(|s| s.from < to && from < s.to).collect();
    let scan_from = shown.first().map_or(from, |s| s.from.min(from));
    let scan_to = shown.last().map_or(to, |s| s.to.max(to));
    let traces: Vec<Trace> = (0..plan.positions.len()).filter_map(|k| trace(scene, &facts, k, scan_from, scan_to)).collect();
    frame_pass(scene, &facts, from, to, opts, &mut m);
    for t in traces.iter().filter(|t| t.span.start_frame < to && from <= t.span.end_frame) {
        let (Some(p), Some(dep)) = (plan.positions.get(t.k), plan.positions.get(t.span.from)) else { continue };
        let n = &t.numbers;
        let mean_speed = t.distance / f64::from(t.span.frames.max(1));
        m.moves.push(MoveMeasure {
            position: p.id,
            from: dep.id,
            start: t.span.start_frame,
            end: t.span.end_frame,
            frames: t.span.frames,
            distance: t.distance,
            mean_speed,
            peak_speed: n.peak_speed,
            peak_accel: n.peak_accel,
            start_accel: n.start_accel,
            end_accel: n.end_accel,
            peak_jerk: n.peak_jerk,
            peak_zoom_rate: n.peak_zoom_rate,
        });
        if th.max_accel.is_some_and(|max| n.peak_accel > max) {
            let detail = json!({"position": p.id, "peakAccel": n.peak_accel, "maxAccel": th.max_accel});
            m.findings.push(issue("accel_over", Level::Warning, Some(plan.id), Some([t.span.start_frame, t.span.end_frame + 1]), detail));
        }
        if th.min_move_distance.is_some_and(|min| t.distance < min) {
            let detail = json!({"position": p.id, "distance": t.distance, "minMoveDistance": th.min_move_distance});
            m.findings.push(issue("small_move", Level::Warning, Some(plan.id), Some([t.span.start_frame, t.span.end_frame + 1]), detail));
        }
    }
    let range = [from, to];
    junctions(scene, &facts, &traces, &th, range, &mut m);
    breaks(scene, &facts, &th, range, &mut m);
    shot_measures(&facts, &shown, &traces, &th, &mut m);
    positions_findings(scene, &facts, &th, range, &mut m);
    labels(scene, &facts, range, &mut m);
    parts(scene, &facts, range, &mut m);
    regions(scene, plan, range, opts, &mut m);
    m.totals.carried_shots = u32::try_from(shown.iter().filter(|s| s.status != ShotStatus::Own).count()).unwrap_or(u32::MAX);
    m
}

/// The trace of the move into position `k` when it touches `[from, to)`.
fn trace(scene: &Scene, facts: &Facts, k: usize, from: i64, to: i64) -> Option<Trace> {
    let span = facts.spans.get(k).copied().flatten()?;
    if span.start_frame >= to || span.end_frame < from {
        return None;
    }
    let plan = facts.plan;
    let (a, b) = (plan.positions.get(span.from)?, plan.positions.get(span.to)?);
    let numbers = move_numbers_of(plan, &scene.r, &span);
    let distance = corner_deltas(&a.window, &b.window, plan.output).map_or(0.0, |d| speed_of(&d));
    Some(Trace { k, span, numbers, distance, shift: [b.window.cx - a.window.cx, b.window.cy - a.window.cy] })
}

/// The per-frame pass over `[from, to)`: the series, the totals, sustained speed, `speed_over` and `upscale_over`.
fn frame_pass(scene: &Scene, facts: &Facts, from: i64, to: i64, opts: &MeasureOpts, m: &mut Measures) {
    let (r, plan) = (scene.r, facts.plan);
    let out = plan.output;
    let th = opts.thresholds;
    let mut series = opts.series.then(Series::default);
    let mut prev: Option<Sample> = (from > 0).then(|| window_at(plan, &r, r.rate.tick_of(from - 1)));
    let mut prev_delta = STILL;
    let (mut min_margin, mut max_upscale) = (f64::INFINITY, 0.0_f64);
    let mut fast: Option<(i64, f64)> = None;
    let mut sustained: i64 = 0;
    let mut zoomed: Option<(i64, f64)> = None;
    let mut f = from;
    while f < to {
        let s = window_at(plan, &r, r.rate.tick_of(f));
        let motion = moves_at(facts, f, &s);
        let delta = match prev {
            Some(p) if motion && p.layout == Layout::Fill => corner_deltas(&p.window, &s.window, out).unwrap_or(STILL),
            _ => STILL,
        };
        let speed = speed_of(&delta);
        let upscale = upscale_of(&s, &r, out);
        max_upscale = max_upscale.max(upscale);
        if s.layout == Layout::Fill {
            let rect = s.window.rect(out);
            let margin = rect.x.min(rect.y).min(f64::from(r.width) - rect.x - rect.w).min(f64::from(r.height) - rect.y - rect.h);
            min_margin = min_margin.min(margin);
        }
        if let Some(max) = th.max_speed {
            fast = run_step(fast, f, speed, speed > max, |a, b, peak| {
                sustained = sustained.max(b - a);
                let detail = json!({"fromFrame": a, "toFrame": b, "peak": peak, "maxSpeed": max});
                m.findings.push(issue("speed_over", Level::Warning, Some(plan.id), Some([a, b]), detail));
            });
        }
        if let Some(max) = th.max_upscale {
            zoomed = run_step(zoomed, f, upscale, upscale > max, |a, b, peak| {
                let detail = json!({"fromFrame": a, "toFrame": b, "upscale": peak, "maxUpscale": max});
                m.findings.push(issue("upscale_over", Level::Warning, Some(plan.id), Some([a, b]), detail));
            });
        }
        if let Some(series) = series.as_mut() {
            let zoom = match prev {
                Some(p) if motion => zoom_rate(p.window.h, s.window.h),
                _ => 0.0,
            };
            series.frames.push(f);
            series.speed.push(speed);
            series.accel.push(accel_of(&delta, &prev_delta));
            series.zoom_rate.push(zoom);
            series.upscale.push(upscale);
            series.bar_pixels.push(bar_pixels(scene, plan, f, &s));
        }
        prev_delta = delta;
        prev = Some(s);
        f += 1;
    }
    if let Some(max) = th.max_speed {
        if let Some((a, peak)) = fast {
            sustained = sustained.max(to - a);
            let detail = json!({"fromFrame": a, "toFrame": to, "peak": peak, "maxSpeed": max});
            m.findings.push(issue("speed_over", Level::Warning, Some(plan.id), Some([a, to]), detail));
        }
        m.totals.sustained_speed = Some(sustained);
    }
    if let (Some(max), Some((a, peak))) = (th.max_upscale, zoomed) {
        let detail = json!({"fromFrame": a, "toFrame": to, "upscale": peak, "maxUpscale": max});
        m.findings.push(issue("upscale_over", Level::Warning, Some(plan.id), Some([a, to]), detail));
    }
    m.totals.frames = to - from;
    m.totals.min_margin = if min_margin.is_finite() { min_margin } else { 0.0 };
    m.totals.max_upscale = max_upscale;
    m.series = series;
}

/// One frame of a run of frames over a threshold: starts, extends (keeping the peak value) or ends the run, calling
/// `close(from, to, peak)` when it ends.
fn run_step(run: Option<(i64, f64)>, f: i64, value: f64, over: bool, mut close: impl FnMut(i64, i64, f64)) -> Option<(i64, f64)> {
    match (run, over) {
        (Some((a, peak)), true) => Some((a, peak.max(value))),
        (None, true) => Some((f, value)),
        (Some((a, peak)), false) => {
            close(a, f, peak);
            None
        }
        (None, false) => None,
    }
}

/// The output pixels of a frame that show the bars outside the active area.
fn bar_pixels(scene: &Scene, plan: &VerticalPlan, frame: i64, s: &Sample) -> f64 {
    let r = scene.r;
    let Some(set) = scene.set else { return 0.0 };
    let [ax, ay, aw, ah] = active_rect(set, &r, r.rate.tick_of(frame));
    let (shown, k) = match s.layout {
        Layout::Fill => (s.window.rect(plan.output), s.window.h / f64::from(plan.output.height)),
        Layout::Fit => {
            let frame_rect = Rect { x: 0.0, y: 0.0, w: f64::from(r.width), h: f64::from(r.height) };
            (frame_rect, f64::from(r.width) / f64::from(plan.output.width.max(1)))
        }
    };
    let ix = (shown.x + shown.w).min(ax + aw) - shown.x.max(ax);
    let iy = (shown.y + shown.h).min(ay + ah) - shown.y.max(ay);
    let inter = ix.max(0.0) * iy.max(0.0);
    let bars = shown.w * shown.h - inter;
    if bars <= 1e-9 || k <= 0.0 { 0.0 } else { bars / (k * k) }
}

/// The junctions between consecutive moves (`s_{k+1} = f_k`) inside the range, with `speed_step_over`.
fn junctions(scene: &Scene, facts: &Facts, traces: &[Trace], th: &Thresholds, range: [i64; 2], m: &mut Measures) {
    let (r, plan) = (scene.r, facts.plan);
    for pair in traces.windows(2) {
        let [a, b] = pair else { continue };
        let f = a.span.end_frame;
        if b.span.start_frame != f || f < range[0] || f >= range[1] {
            continue;
        }
        let (Some(into), Some(out_of)) = (plan.positions.get(a.k), plan.positions.get(b.k)) else { continue };
        let at = |frame: i64| window_at(plan, &r, r.rate.tick_of(frame));
        let (s0, s1, s2) = (at(f - 1), at(f), at(f + 1));
        let v_in = arriving_speed(scene, facts, f, &s1);
        let v_out = arriving_speed(scene, facts, f + 1, &s2);
        let arriving = [s1.window.cx - s0.window.cx, s1.window.cy - s0.window.cy];
        let leaving = [s2.window.cx - s1.window.cx, s2.window.cy - s1.window.cy];
        let speed_step = (v_out - v_in).abs();
        m.junctions.push(JunctionMeasure { frame: f, into: into.id, out_of: out_of.id, speed_step, turn: turn(arriving, leaving) });
        if let Some(max) = th.max_speed_step.filter(|&max| speed_step > max) {
            let detail = json!({"frame": f, "into": into.id, "outOf": out_of.id, "speedStep": speed_step, "maxSpeedStep": max});
            m.findings.push(issue("speed_step_over", Level::Warning, Some(plan.id), Some([f, f + 1]), detail));
        }
    }
}

/// Whether the window moves arriving at `frame`: a frame inside a valid move or the one it arrives at.
fn moves_at(facts: &Facts, frame: i64, s: &Sample) -> bool {
    s.layout == Layout::Fill && (s.phase == Phase::Move || facts.arrives_moving(frame))
}

/// The centre's displacement arriving at `frame` while the window moves there; None otherwise.
fn centre_motion(scene: &Scene, facts: &Facts, frame: i64) -> Option<[f64; 2]> {
    let (r, plan) = (scene.r, facts.plan);
    if frame <= 0 || frame >= r.end_frame {
        return None;
    }
    let s = window_at(plan, &r, r.rate.tick_of(frame));
    if !moves_at(facts, frame, &s) {
        return None;
    }
    let before = window_at(plan, &r, r.rate.tick_of(frame - 1));
    Some([s.window.cx - before.window.cx, s.window.cy - before.window.cy])
}

/// The focus a frame shows: its governing position's focus point, mapped onto the output.
fn focus_on(scene: &Scene, plan: &VerticalPlan, frame: i64) -> Option<[f64; 2]> {
    let r = scene.r;
    let id = governing(plan, &r, frame)?;
    let point = plan.positions.iter().find(|p| p.id == id)?.focus?;
    let s = window_at(plan, &r, r.rate.tick_of(frame));
    Some(output_point(&s, &r, plan.output, point))
}

/// The jumps and the window breaks inside the range, with `jump_similar`, `eye_trace_jump` and `cue_straddles_cut`.
fn breaks(scene: &Scene, facts: &Facts, th: &Thresholds, range: [i64; 2], m: &mut Measures) {
    let (r, plan) = (scene.r, facts.plan);
    let (ow, oh) = (f64::from(plan.output.width), f64::from(plan.output.height));
    let cues = cue_frames(scene, Some(plan));
    let id = Some(plan.id);
    for (k, p) in plan.positions.iter().enumerate() {
        let tag = facts.tag(k);
        let c = facts.frame(k);
        if !matches!(tag, Tag::Cut | Tag::Jump) || c < range[0] || c >= range[1] || c <= 0 {
            continue;
        }
        let before = window_at(plan, &r, r.rate.tick_of(c - 1));
        let after = window_at(plan, &r, r.rate.tick_of(c));
        let at = Some([c, c + 1]);
        if tag == Tag::Jump {
            let (b, a) = (before.window, after.window);
            let ka = a.h / oh;
            let (dx, dy) = ((b.cx - a.cx) / ka, (b.cy - a.cy) / ka);
            let shift = (dx * dx + dy * dy).sqrt() / ow;
            let scale = b.h.max(a.h) / b.h.min(a.h);
            let iou = overlap(&b.rect(plan.output), &a.rect(plan.output));
            m.jumps.push(JumpMeasure { position: p.id, frame: c, shift, scale, iou });
            if let Some(similar) = th.jump_similar.filter(|j| shift < j.max_shift && scale < j.max_scale) {
                let (max_shift, max_scale) = (similar.max_shift, similar.max_scale);
                let detail = json!({"position": p.id, "frame": c, "shift": shift, "scale": scale, "maxShift": max_shift, "maxScale": max_scale});
                m.findings.push(issue("jump_similar", Level::Warning, id, at, detail));
            }
        }
        let eye_trace = match (focus_on(scene, plan, c - 1), focus_on(scene, plan, c)) {
            (Some(o0), Some(o1)) => {
                let (dx, dy) = (o1[0] - o0[0], o1[1] - o0[1]);
                Some(EyeTraceMeasure { before: o0, after: o1, distance: (dx * dx + dy * dy).sqrt() / ow })
            }
            _ => None,
        };
        let kind = if tag == Tag::Cut { BreakKind::Cut } else { BreakKind::Jump };
        if let (Some(trace), Some(max)) = (eye_trace, th.max_eye_trace_jump)
            && trace.distance > max
        {
            let detail = json!({"position": p.id, "frame": c, "kind": kind, "distance": trace.distance, "maxEyeTraceJump": max});
            m.findings.push(issue("eye_trace_jump", Level::Warning, id, at, detail));
        }
        let direction = match (centre_motion(scene, facts, c - 1), centre_motion(scene, facts, c + 1)) {
            (Some(a), Some(b)) => direction(a[0], a[1]).zip(direction(b[0], b[1])).map(|(x, y)| [x, y]),
            _ => None,
        };
        let cue_start = nearest(cues.iter().map(|q| q.0), c);
        let cue_end = nearest(cues.iter().map(|q| q.1), c);
        if let Some(n) = th.straddle_frames {
            for &(first, end, cue, _) in cues.iter().filter(|q| q.0 < c && q.1 > c) {
                let (b, a) = (c.saturating_sub(first), end.saturating_sub(c));
                if b <= n || a <= n {
                    let detail = json!({"position": p.id, "frame": c, "cue": cue, "before": b, "after": a});
                    m.findings.push(issue("cue_straddles_cut", Level::Warning, id, at, detail));
                }
            }
        }
        m.breaks.push(BreakMeasure { position: p.id, frame: c, kind, eye_trace, direction, cue_start, cue_end });
    }
}

/// The intersection over union of two rects.
fn overlap(a: &Rect, b: &Rect) -> f64 {
    let ix = ((a.x + a.w).min(b.x + b.w) - a.x.max(b.x)).max(0.0);
    let iy = ((a.y + a.h).min(b.y + b.h) - a.y.max(b.y)).max(0.0);
    let inter = ix * iy;
    let union = a.w * a.h + b.w * b.h - inter;
    if union > 0.0 { inter / union } else { 0.0 }
}

/// The shots overlapping the range, with `early_move` and `ping_pong`.
fn shot_measures(facts: &Facts, shown: &[&Shot], traces: &[Trace], th: &Thresholds, m: &mut Measures) {
    let plan = facts.plan;
    let id = Some(plan.id);
    for shot in shown {
        let ks: Vec<usize> = (0..plan.positions.len()).filter(|&k| (shot.from..shot.to).contains(&facts.frame(k))).collect();
        let tagged = |tag: Tag| ks.iter().filter(|&&k| facts.tag(k) == tag).count();
        let jumps: Vec<i64> = ks.iter().filter(|&&k| facts.tag(k) == Tag::Jump).map(|&k| facts.frame(k)).collect();
        let moves: Vec<&Trace> = traces.iter().filter(|t| (shot.from..shot.to).contains(&t.span.end_frame)).collect();
        let settle = moves.first().map_or(0, |first| {
            let mut end = first.span.end_frame;
            for t in moves.iter().skip(1) {
                if t.span.start_frame != end {
                    break;
                }
                end = t.span.end_frame;
            }
            end - shot.from
        });
        let frames = shot.to.saturating_sub(shot.from).max(1);
        let moving: i64 = moves.iter().map(|t| i64::from(t.numbers.moving_frames)).sum();
        let still_share = (frames - moving).max(0) as f64 / frames as f64;
        let first_moving = moves.iter().find_map(|t| t.numbers.first_moving.map(|k| t.span.start_frame + i64::from(k)));
        let hold_after_wall = first_moving.map(|f| f - shot.from);
        let gaps: Vec<i64> = jumps.windows(2).map(|w| w.get(1).copied().unwrap_or(0) - w.first().copied().unwrap_or(0)).collect();
        let jump_rhythm = gaps.iter().min().zip(gaps.iter().max()).map(|(a, b)| [*a, *b]);
        let mut reversals = Vec::new();
        if let Some(n) = th.ping_pong_frames {
            let mut last: Option<([f64; 2], i64)> = None;
            for t in moves.iter().filter(|t| t.shift != [0.0, 0.0]) {
                let (Some(first), Some(end)) = (t.numbers.first_moving, t.numbers.last_moving) else { continue };
                let start = t.span.start_frame + i64::from(first);
                if let Some((dir, frame)) = last {
                    let dot = dir[0] * t.shift[0] + dir[1] * t.shift[1];
                    if dot < 0.0 && start - frame <= n {
                        reversals.push(start);
                        let detail = json!({"shot": shot.number, "frame": start, "frames": start - frame, "pingPongFrames": n});
                        m.findings.push(issue("ping_pong", Level::Warning, id, Some([start, start + 1]), detail));
                    }
                }
                last = Some((t.shift, t.span.start_frame + i64::from(end)));
            }
        }
        if let (Some(min), Some(hold)) = (th.min_hold_after_wall, hold_after_wall)
            && shot.number > 1
            && hold < min
        {
            let position = moves.first().and_then(|t| plan.positions.get(t.k)).map(|p| p.id);
            let detail = json!({"shot": shot.number, "position": position, "frames": hold, "minHoldAfterWall": min});
            m.findings.push(issue("early_move", Level::Warning, id, Some([shot.from, shot.from + hold + 1]), detail));
        }
        m.shots.push(ShotMeasure {
            number: shot.number,
            from: shot.from,
            to: shot.to,
            status: shot.status,
            positions: u32::try_from(ks.len()).unwrap_or(u32::MAX),
            moves: u32::try_from(tagged(Tag::Move)).unwrap_or(u32::MAX),
            jumps: u32::try_from(tagged(Tag::Jump)).unwrap_or(u32::MAX),
            settle,
            still_share,
            hold_after_wall,
            jump_rhythm,
            reversals,
        });
    }
}

/// `short_hold`, `long_hold` and `near_wall_change` for the positions in the range.
fn positions_findings(scene: &Scene, facts: &Facts, th: &Thresholds, range: [i64; 2], m: &mut Measures) {
    let plan = facts.plan;
    let id = Some(plan.id);
    let end = scene.r.end_frame;
    for (k, p) in plan.positions.iter().enumerate() {
        let f = facts.frame(k);
        if f < range[0] || f >= range[1] {
            continue;
        }
        let still = facts.still(k, end);
        let hold = still[1] - still[0];
        if let Some(min) = th.min_hold_frames.filter(|&min| hold < min) {
            m.findings.push(issue("short_hold", Level::Warning, id, Some(still), json!({"position": p.id, "holdFrames": hold, "minHoldFrames": min})));
        }
        if let Some(max) = th.max_hold_frames.filter(|&max| hold > max) {
            m.findings.push(issue("long_hold", Level::Warning, id, Some(still), json!({"position": p.id, "holdFrames": hold, "maxHoldFrames": max})));
        }
        let Some(n) = th.near_wall_frames else { continue };
        let changes: Vec<i64> = match facts.tag(k) {
            Tag::Cut | Tag::Jump => vec![f],
            Tag::Move => facts.spans.get(k).copied().flatten().map(|s| vec![s.start_frame + 1, f]).unwrap_or_default(),
            Tag::Start | Tag::Same => Vec::new(),
        };
        for x in changes {
            let Some(d) = nearest(scene.wall_frames.iter().copied(), x) else { continue };
            let distance = -d;
            if distance != 0 && distance.unsigned_abs() <= n.unsigned_abs() {
                let detail = json!({"position": p.id, "frame": x, "wall": x + d, "distance": distance});
                m.findings.push(issue("near_wall_change", Level::Warning, id, Some([x, x + 1]), detail));
            }
        }
    }
}

/// The framing spread of each label over the positions in the range.
fn labels(scene: &Scene, facts: &Facts, range: [i64; 2], m: &mut Measures) {
    let plan = facts.plan;
    let in_range = |k: usize| (range[0]..range[1]).contains(&facts.frame(k));
    let mut order: Vec<&str> = Vec::new();
    for (k, p) in plan.positions.iter().enumerate() {
        if !p.label.is_empty() && in_range(k) && !order.contains(&p.label.as_str()) {
            order.push(&p.label);
        }
    }
    for label in order {
        let mine: Vec<&Position> = plan.positions.iter().enumerate().filter(|(k, p)| p.label == label && in_range(*k)).map(|(_, p)| p).collect();
        let column = |f: fn(&Window) -> f64| mine.iter().map(|p| f(&p.window)).collect::<Vec<f64>>();
        let point = |p: &&Position| {
            let own = Sample { window: p.window, layout: p.layout, ..Sample::default() };
            p.focus.map(|q| output_point(&own, &scene.r, plan.output, q))
        };
        let points: Vec<[f64; 2]> = mine.iter().filter_map(point).collect();
        let focus = (!points.is_empty()).then(|| {
            let (xs, ys): (Vec<f64>, Vec<f64>) = points.iter().map(|q| (q[0], q[1])).unzip();
            [deviation(&xs), deviation(&ys)]
        });
        m.labels.push(LabelSpread {
            label: label.to_string(),
            positions: u32::try_from(mine.len()).unwrap_or(u32::MAX),
            cx: deviation(&column(|w| w.cx)),
            cy: deviation(&column(|w| w.cy)),
            h: deviation(&column(|w| w.h)),
            focus,
        });
    }
}

/// Fit share, fit changes and caption-anchor flips per part overlapping the range.
fn parts(scene: &Scene, facts: &Facts, range: [i64; 2], m: &mut Measures) {
    let (r, plan) = (scene.r, facts.plan);
    let rate = r.rate.sane();
    let cues = cue_frames(scene, Some(plan));
    for part in scene.set.iter().flat_map(|s| &s.parts) {
        let lo = r.rate.frame_at(part.from).max(range[0]);
        let hi = r.rate.frame_at(part.to).min(range[1]);
        if lo >= hi {
            continue;
        }
        let (mut fit, mut changes) = (0_i64, 0_u32);
        let mut last: Option<Layout> = None;
        for f in lo..hi {
            let layout = window_at(plan, &r, r.rate.tick_of(f)).layout;
            fit += i64::from(layout == Layout::Fit);
            if last.is_some_and(|l| l != layout) {
                changes = changes.saturating_add(1);
            }
            last = Some(layout);
        }
        let anchor = |first: i64| {
            let governing = governing(plan, &r, first).and_then(|g| plan.positions.iter().find(|p| p.id == g));
            governing.and_then(|p| p.caption_anchor).unwrap_or(plan.captions.anchor)
        };
        let anchors: Vec<_> = cues.iter().filter(|q| q.0 >= lo && q.0 < hi).map(|q| anchor(q.0)).collect();
        let flips = anchors.windows(2).filter(|w| w.first() != w.get(1)).count() as f64;
        let frames = (hi - lo) as f64;
        let per_minute = flips * 60.0 * rate.num as f64 / (rate.den as f64 * frames);
        m.parts.push(PartMeasure { part: part.id, fit_share: fit as f64 / frames, fit_changes: changes, anchor_flips_per_minute: per_minute });
    }
}

/// A region as measured: how it names itself, its mode, its frames and its box per frame.
struct Measured<'a> {
    id: String,
    region: Option<u32>,
    label: String,
    mode: Option<RegionMode>,
    frames: [i64; 2],
    shape: Shape<'a>,
}

/// Where a measured region's box comes from.
enum Shape<'a> {
    /// A protected region's fixed rect.
    Fixed([f64; 4]),
    /// A caller's boxes, sorted by frame once per measure (the first box of a frame kept), and whether they interpolate.
    Boxes(Vec<&'a RegionBox>, bool),
}

impl Measured<'_> {
    /// The region's box at `frame`, if it is present there.
    fn rect(&self, walls: &[i64], frame: i64) -> Option<[f64; 4]> {
        match &self.shape {
            Shape::Fixed(rect) => Some(*rect),
            Shape::Boxes(boxes, interpolate) => box_at(boxes, *interpolate, walls, frame),
        }
    }
}

/// A caller's boxes sorted by frame, the first box of a frame kept (see [`RegionInput`]).
fn sorted_boxes(input: &RegionInput) -> Vec<&RegionBox> {
    let mut boxes: Vec<&RegionBox> = input.boxes.iter().collect();
    boxes.sort_by_key(|b| b.frame);
    boxes.dedup_by_key(|b| b.frame);
    boxes
}

/// The box of a caller's region at `frame` from its sorted boxes (see [`RegionInput`]); `walls` are the sorted wall
/// frames.
fn box_at(boxes: &[&RegionBox], interpolate: bool, walls: &[i64], frame: i64) -> Option<[f64; 4]> {
    let i = boxes.partition_point(|b| b.frame <= frame).checked_sub(1)?;
    let here = boxes.get(i)?;
    if here.frame == frame {
        return Some(here.rect);
    }
    let next = boxes.get(i + 1)?;
    // a wall in (a, b], by bisection
    let wall_between = |a: i64, b: i64| walls.partition_point(|&w| w <= a) < walls.partition_point(|&w| w <= b);
    if wall_between(here.frame, frame) {
        return None;
    }
    if !interpolate || wall_between(frame, next.frame) {
        return Some(here.rect);
    }
    let t = frame.saturating_sub(here.frame) as f64 / next.frame.saturating_sub(here.frame) as f64;
    let lerp = |a: f64, b: f64| a + (b - a) * t;
    let (a, b) = (here.rect, next.rect);
    Some([lerp(a[0], b[0]), lerp(a[1], b[1]), lerp(a[2], b[2]), lerp(a[3], b[3])])
}

/// The caller's regions and, with `protected`, the set's protected regions; caption collisions and safe-zone hits.
fn regions(scene: &Scene, plan: &VerticalPlan, range: [i64; 2], opts: &MeasureOpts, m: &mut Measures) {
    let r = scene.r;
    let mut measured: Vec<Measured> = Vec::new();
    for input in &opts.regions {
        let first = input.boxes.iter().map(|b| b.frame).min().unwrap_or(0).max(range[0]);
        let last = input.boxes.iter().map(|b| b.frame).max().map_or(range[0], |f| f.saturating_add(1)).min(range[1]);
        let (id, label) = (input.id.clone(), input.label.clone());
        let shape = Shape::Boxes(sorted_boxes(input), input.interpolate);
        measured.push(Measured { id, region: None, label, mode: None, frames: [first, last], shape });
    }
    if opts.protected {
        for region in scene.set.iter().flat_map(|s| &s.regions) {
            let frames = [r.rate.frame_at(region.from).max(range[0]), r.rate.frame_at(region.to).min(range[1])];
            let (label, mode) = (region.label.clone(), Some(region.mode));
            measured.push(Measured { id: String::new(), region: Some(region.id), label, mode, frames, shape: Shape::Fixed(region.rect) });
        }
    }
    for reg in &measured {
        let (one, warning) = measure_region(scene, plan, reg, opts.safe_zone);
        m.regions.push(one);
        m.findings.extend(warning);
    }
    for caption in &opts.captions {
        for reg in &measured {
            let (a, b) = (caption.from_frame.max(reg.frames[0]), caption.to_frame.min(reg.frames[1]));
            let mut hits: Vec<[i64; 2]> = Vec::new();
            for f in a..b {
                let Some(rect) = reg.rect(&scene.wall_frames, f) else { continue };
                let s = window_at(plan, &r, r.rate.tick_of(f));
                let shown = clip_to_output(output_rect(&s, &r, plan.output, rect), plan.output);
                if shown.is_some_and(|o| intersects(&o, &caption.rect)) {
                    match hits.last_mut() {
                        Some(run) if run[1] == f => run[1] = f + 1,
                        _ => hits.push([f, f + 1]),
                    }
                }
            }
            if let (Some(first), Some(last)) = (hits.first().copied(), hits.last().copied()) {
                let region = reg.region.map_or_else(|| json!(reg.id), |id| json!(id));
                let detail = json!({"cue": caption.cue, "region": region, "frames": hits});
                m.findings.push(issue("caption_collision", Level::Warning, Some(plan.id), Some([first[0], last[1]]), detail));
            }
        }
    }
}

/// The measure of one region over its frames, and its warning when it is a protected region whose mode is broken.
fn measure_region(scene: &Scene, plan: &VerticalPlan, reg: &Measured, safe_zone: Option<[f64; 4]>) -> (RegionMeasure, Option<Issue>) {
    let r = scene.r;
    let out = plan.output;
    let mut states: Vec<(i64, RegionState, f64)> = Vec::new();
    let (mut frames_in, mut frames_sliced, mut frames_out, mut hits) = (0_u32, 0_u32, 0_u32, 0_u32);
    let mut worst: Option<(i64, f64, [f64; 4])> = None;
    for f in reg.frames[0]..reg.frames[1] {
        let Some(rect) = reg.rect(&scene.wall_frames, f) else { continue };
        let s = window_at(plan, &r, r.rate.tick_of(f));
        let (state, fraction) = visible_in(&s, out, rect);
        match state {
            RegionState::In => frames_in = frames_in.saturating_add(1),
            RegionState::Sliced => frames_sliced = frames_sliced.saturating_add(1),
            RegionState::Out => frames_out = frames_out.saturating_add(1),
        }
        let worse = match worst {
            None => true,
            Some((_, w, _)) if reg.mode == Some(RegionMode::KeepOut) => fraction > w,
            Some((_, w, _)) => fraction < w,
        };
        if worse {
            worst = Some((f, fraction, rect));
        }
        if let Some(zone) = safe_zone {
            let shown = clip_to_output(output_rect(&s, &r, out, rect), out);
            if shown.is_some_and(|o| !inside(&o, &zone)) {
                hits = hits.saturating_add(1);
            }
        }
        states.push((f, state, fraction));
    }
    let runs = runs_of(states.into_iter());
    let margins = worst.map(|(f, _, rect)| margins_at(&window_at(plan, &r, r.rate.tick_of(f)), &r, out, rect));
    let warning = reg.mode.zip(reg.region).and_then(|(mode, region)| mode_warning(plan.id, region, mode, &runs));
    let one = RegionMeasure {
        id: reg.id.clone(),
        region: reg.region,
        label: reg.label.clone(),
        mode: reg.mode,
        frames_in,
        frames_sliced,
        frames_out,
        runs,
        worst_frame: worst.map(|w| w.0),
        margins,
        safe_zone_hits: hits,
    };
    (one, warning)
}

/// Output px from the box to the edges of what a frame shows (the window in fill, the whole frame in fit): left, top,
/// right, bottom; negative where the edge cuts the box.
fn margins_at(s: &Sample, r: &Raster, out: OutputSize, rect: [f64; 4]) -> [f64; 4] {
    let (shown, k) = match s.layout {
        Layout::Fill => (s.window.rect(out), s.window.h / f64::from(out.height)),
        Layout::Fit => (Rect { x: 0.0, y: 0.0, w: f64::from(r.width), h: f64::from(r.height) }, f64::from(r.width) / f64::from(out.width.max(1))),
    };
    let [x, y, w, h] = rect;
    [(x - shown.x) / k, (y - shown.y) / k, (shown.x + shown.w - x - w) / k, (shown.y + shown.h - y - h) / k]
}

/// The part of an output rect inside the output frame; None when nothing of it is.
fn clip_to_output(rect: [f64; 4], out: OutputSize) -> Option<[f64; 4]> {
    let (x0, y0) = (rect[0].max(0.0), rect[1].max(0.0));
    let (x1, y1) = ((rect[0] + rect[2]).min(f64::from(out.width)), (rect[1] + rect[3]).min(f64::from(out.height)));
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
}

/// Whether two `[x, y, w, h]` rects overlap.
fn intersects(a: &[f64; 4], b: &[f64; 4]) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

/// Whether `a` lies inside `b`.
fn inside(a: &[f64; 4], b: &[f64; 4]) -> bool {
    a[0] >= b[0] && a[1] >= b[1] && a[0] + a[2] <= b[0] + b[2] && a[1] + a[3] <= b[1] + b[3]
}
