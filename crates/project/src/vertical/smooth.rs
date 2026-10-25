//! Smooth through (spec section 3.7): a run of moves through fixed waypoints made to read as one camera move, with the
//! screen speed continuous at every waypoint, an ease in from rest (or a moving start) and an ease out to rest (or a moving
//! end). It keeps every waypoint's time and window and returns only the new arrivals.

use std::collections::BTreeMap;

use serde_json::json;

use super::canon::floor_micro;
use super::path::{end_rate, start_rate};
use super::types::{Arrival, Curve, Layout, OutputSize, Position, Raster, Refusal, SmoothOpts, VerticalPlan, Window};

/// `V(delta; at)`: the screen speed, in output pixels, of the window change `delta` (in `cx`, `cy` and `h`) made at the
/// window `at`. The largest length over the four output corners `o` of `-o * dh / h - (dcx - dh * OW / (2 * OH),
/// dcy - dh / 2) * OH / h`.
pub fn screen_speed(delta: Window, at: Window, out: OutputSize) -> f64 {
    let (ow, oh) = (f64::from(out.width), f64::from(out.height));
    let gx = (delta.cx - delta.h * ow / (2.0 * oh)) * oh / at.h;
    let gy = (delta.cy - delta.h / 2.0) * oh / at.h;
    let corner = |(ox, oy): (f64, f64)| {
        let (dx, dy) = (-(ox * delta.h / at.h) - gx, -(oy * delta.h / at.h) - gy);
        (dx * dx + dy * dy).sqrt()
    };
    [(0.0, 0.0), (ow, 0.0), (0.0, oh), (ow, oh)].into_iter().map(corner).fold(0.0, f64::max)
}

/// The arrivals of the positions `ids[1..]` that make the run `ids` one continuous move: every move takes its whole gap
/// with a `speeds` curve. `ids` names at least 3 consecutive positions, all `fill`, none locked, at least 2 frames apart;
/// the planner checks the walls. Refusals: `not_found`, `out_of_range`, `layout_move`, `locked`, `move_too_short`.
pub fn smooth_through(plan: &VerticalPlan, r: &Raster, ids: &[u32], opts: SmoothOpts) -> Result<Vec<Arrival>, Refusal> {
    let run = run_of(plan, ids)?;
    let fit: Vec<u32> = run.iter().filter(|p| p.layout != Layout::Fill).map(|p| p.id).collect();
    if !fit.is_empty() {
        return Err(Refusal::new("layout_move", "a move never arrives at or leaves a fit position", json!({"ids": fit})));
    }
    let locked: Vec<u32> = run.iter().filter(|p| p.locked).map(|p| p.id).collect();
    if !locked.is_empty() {
        return Err(Refusal::new("locked", "the run holds locked positions; unlock them first", json!({"ids": locked})));
    }
    let mut segments = Vec::with_capacity(run.len());
    for pair in run.windows(2) {
        let [from, to] = pair else { continue };
        segments.push(Segment::new(from, to, r, plan.output)?);
    }
    // the end speed of the move into each inner waypoint and the start speed of the move out of it
    let last = segments.len().saturating_sub(2);
    let mut joins = Vec::with_capacity(segments.len());
    for (k, (into, out_of)) in segments.iter().zip(segments.iter().skip(1)).enumerate() {
        joins.push(junction(into, out_of, k == 0, k == last, opts));
    }
    let start = if opts.start_from_rest { 0.0 } else { 1.0 };
    let end = if opts.end_at_rest { 0.0 } else { 1.0 };
    let arrival = |(k, segment): (usize, &Segment)| {
        let alpha = k.checked_sub(1).and_then(|j| joins.get(j)).map_or(start, |join| join.1);
        let beta = joins.get(k).map_or(end, |join| join.0);
        Arrival::Move { move_frames: segment.frames, curve: Curve::Speeds { start: alpha, end: beta }, split_of: None }
    };
    Ok(segments.iter().enumerate().map(arrival).collect())
}

/// The positions `ids` names, when they are at least 3 consecutive positions in plan order.
fn run_of<'a>(plan: &'a VerticalPlan, ids: &[u32]) -> Result<&'a [Position], Refusal> {
    let index: BTreeMap<u32, usize> = plan.positions.iter().enumerate().map(|(i, p)| (p.id, i)).collect();
    if let Some(&id) = ids.iter().find(|&&id| !index.contains_key(&id)) {
        return Err(Refusal::new("not_found", format!("the plan has no position {id}"), json!({"position": id})));
    }
    let first = ids.first().and_then(|id| index.get(id)).copied().unwrap_or(0);
    let consecutive = ids.iter().enumerate().all(|(k, id)| index.get(id).copied() == first.checked_add(k));
    match plan.positions.get(first..first.saturating_add(ids.len())) {
        Some(run) if consecutive && run.len() >= 3 => Ok(run),
        _ => Err(Refusal::new("out_of_range", "Smooth through needs at least 3 consecutive positions", json!({"count": ids.len(), "minimum": 3}))),
    }
}

/// What Smooth through needs of one move of the run: its length, its screen speed per unit of curve speed where it leaves
/// its first waypoint (`B`) and where it reaches its last (`A`), and its mean screen speed, all in output pixels per frame.
#[derive(Clone, Copy, Debug)]
struct Segment {
    frames: u32,
    leave: f64,
    arrive: f64,
    mean: f64,
}

impl Segment {
    /// The move from `from` to `to` over their whole gap (steps 1-4 of spec section 3.7). Refused `move_too_short` when the
    /// gap is under 2 frames, and `out_of_range` when it is longer than a move can be.
    fn new(from: &Position, to: &Position, r: &Raster, out: OutputSize) -> Result<Self, Refusal> {
        let gap = r.rate.frame_at(to.at).saturating_sub(r.rate.frame_at(from.at));
        if gap < 2 {
            let detail = json!({"position": to.id, "frames": gap, "minimum": 2});
            return Err(Refusal::new("move_too_short", "every waypoint must be at least 2 frames after the one before", detail));
        }
        let Ok(frames) = u32::try_from(gap) else {
            let detail = json!({"position": to.id, "frames": gap, "maximum": u32::MAX});
            return Err(Refusal::new("out_of_range", "the gap is longer than a move can be", detail));
        };
        let (a, b) = (&from.window, &to.window);
        let delta = Window { cx: b.cx - a.cx, cy: b.cy - a.cy, h: b.h - a.h };
        let (at_start, at_end) = (screen_speed(delta, *a, out), screen_speed(delta, *b, out));
        let d = f64::from(frames);
        let leave = at_start * start_rate(a.h, b.h) / d;
        let arrive = at_end * end_rate(a.h, b.h) / d;
        Ok(Self { frames, leave, arrive, mean: (at_start + at_end) / (2.0 * d) })
    }
}

/// The end speed `β` of `into` and the start speed `α` of `out_of` at the waypoint between them, which give both moves the
/// junction speed `v` on screen (steps 4-7 of spec section 3.7). The ease in from rest floors the first junction and the
/// ease out to rest the last, so those moves speed up or slow down all the way; the caps keep every curve valid.
fn junction(into: &Segment, out_of: &Segment, first: bool, last: bool, opts: SmoothOpts) -> (f64, f64) {
    let (a, b) = (into.arrive, out_of.leave);
    // a waypoint with no movement on either side is a stop
    let moving = a > 0.0 && b > 0.0 && a.is_finite() && b.is_finite();
    if !moving {
        return (0.0, 0.0);
    }
    let target = (into.mean + out_of.mean) / 2.0;
    let floor_in = if first && opts.start_from_rest { 1.5 * a } else { 0.0 };
    let floor_out = if last && opts.end_at_rest { 1.5 * b } else { 0.0 };
    let cap_end = if first { 3.0 } else { 1.5 };
    let cap_start = if last { 3.0 } else { 1.5 };
    let v = target.max(floor_in.max(floor_out)).min((cap_end * a).min(cap_start * b));
    (floor_micro(v / a), floor_micro(v / b))
}
