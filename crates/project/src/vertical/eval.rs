//! The evaluator (spec section 3.1): the exact window of a plan at any tick. Every preview, read and export calls
//! [`window_at`]; there is no second implementation.

use filmcraft_time::Tick;

use super::canon::{ceil_g, floor_g};
use super::curve::ease;
use super::path::path;
use super::types::{Arrival, Curve, Layout, Limits, OutputSize, Phase, Position, Raster, Sample, SplitOf, VerticalPlan, Window};

/// The move into one position as the evaluator sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveSpan {
    /// The index of the position the move departs from.
    pub from: usize,
    /// The index of the position the move arrives at.
    pub to: usize,
    /// `s = max(tick_of(f_j - d), at_i)`: the start tick, which shows the departing window.
    pub start: Tick,
    pub start_frame: i64,
    /// `at_j`: the arriving position's tick.
    pub end: Tick,
    pub end_frame: i64,
    /// `d`: the move's length in frames.
    pub frames: u32,
    pub curve: Curve,
    pub split: Option<SplitOf>,
}

/// The heights a plan of `out` allows on `r`, both rounded inward onto the grid: `h_max = min(H, W * OH / OW)` and
/// `h_min = OH / 4`.
pub fn limits(r: &Raster, out: OutputSize) -> Limits {
    let (w, h) = (f64::from(r.width), f64::from(r.height));
    let (ow, oh) = (f64::from(out.width), f64::from(out.height));
    Limits { min_height: ceil_g(oh / 4.0), max_height: floor_g(h.min(w * oh / ow)) }
}

/// The centred, largest window: a plan's window while it has no position and no starting window.
pub fn centred_largest(r: &Raster, out: OutputSize) -> Window {
    Window { cx: f64::from(r.width) / 2.0, cy: f64::from(r.height) / 2.0, h: limits(r, out).max_height }
}

/// The move into `plan.positions[to]`, or None for a cut, the first position, an index past the end, or a move that
/// arrives at or departs from a `fit` position (a layout changes by cut only).
pub fn move_span(plan: &VerticalPlan, r: &Raster, to: usize) -> Option<MoveSpan> {
    let from = to.checked_sub(1)?;
    let (departing, arriving) = (plan.positions.get(from)?, plan.positions.get(to)?);
    let Arrival::Move { move_frames, curve, split_of } = arriving.arrival else { return None };
    if departing.layout != Layout::Fill || arriving.layout != Layout::Fill {
        return None;
    }
    let end_frame = r.rate.frame_at(arriving.at);
    // equal to `tick_of(f_j - d)` in valid data (I5); the `max` keeps bad data total
    let start = r.rate.tick_of(end_frame.saturating_sub(i64::from(move_frames))).max(departing.at);
    let start_frame = r.rate.frame_at(start);
    Some(MoveSpan { from, to, start, start_frame, end: arriving.at, end_frame, frames: move_frames, curve, split: split_of })
}

/// The window of `plan` at tick `t` on `r`: the starting window, the first position's window before it, a position's
/// window on and after it, and the path of a move inside it. Exact at positions, bit for bit.
pub fn window_at(plan: &VerticalPlan, r: &Raster, t: Tick) -> Sample {
    let Some(first) = plan.positions.first() else {
        let window = plan.starting_window.unwrap_or_else(|| centred_largest(r, plan.output));
        return Sample { window, layout: Layout::Fill, phase: Phase::Starting, position: None, next: None, u: 0.0, e: 0.0 };
    };
    if t < first.at {
        return still(first, Phase::Before);
    }
    // the last position at or before `t` (I1: positions are sorted by `at`)
    let i = plan.positions.partition_point(|p| p.at <= t).saturating_sub(1);
    let departing = plan.positions.get(i).unwrap_or(first);
    let Some(span) = move_span(plan, r, i + 1).filter(|m| m.start < t) else { return still(departing, Phase::Hold) };
    let Some(arriving) = plan.positions.get(span.to) else { return still(departing, Phase::Hold) };
    let (offset, total) = span.split.map_or((0, span.frames), |split| (split.offset, split.total));
    let total = f64::from(total.max(1));
    // `u` is the piece's own progress; `ub` the progress through the original move a split piece belongs to
    let (u, ub) = if t == r.rate.tick_of(r.rate.frame_at(t)) {
        // a frame start: one division of two small integers
        let done = r.rate.frame_at(t).saturating_sub(span.start_frame);
        (done as f64 / f64::from(span.frames.max(1)), i64::from(offset).saturating_add(done) as f64 / total)
    } else {
        let ratio = ticks_between(span.start, t) / ticks_between(span.start, span.end);
        (ratio, (f64::from(offset) + ratio * f64::from(span.frames)) / total)
    };
    // a piece whose curve does not advance over it (bad data only) moves at constant speed
    let e = match span.split {
        None => ease(&span.curve, ub),
        Some(_) => piece_progress(&span.curve, ub, offset, span.frames, total).unwrap_or(u),
    };
    let window = path(&departing.window, &arriving.window, e);
    Sample { window, layout: Layout::Fill, phase: Phase::Move, position: Some(departing.id), next: Some(arriving.id), u, e }
}

/// The sample that holds `p`'s window and layout.
fn still(p: &Position, phase: Phase) -> Sample {
    Sample { window: p.window, layout: p.layout, phase, position: Some(p.id), next: None, u: 0.0, e: 0.0 }
}

/// The eased progress of a split piece at the original move's progress `ub` (spec section 3.6):
/// `(E(ub) - E(o / T)) / (E((o + d) / T) - E(o / T))`. None when the curve does not advance over the piece (bad data).
fn piece_progress(curve: &Curve, ub: f64, offset: u32, frames: u32, total: f64) -> Option<f64> {
    let e0 = ease(curve, f64::from(offset) / total);
    let e1 = ease(curve, (f64::from(offset) + f64::from(frames)) / total);
    let span = e1 - e0;
    (span > 0.0).then(|| (ease(curve, ub) - e0) / span)
}

/// `b - a` in ticks, as a float; exact for any two ticks less than 2^53 apart.
fn ticks_between(a: Tick, b: Tick) -> f64 {
    (i128::from(b.0) - i128::from(a.0)) as f64
}
