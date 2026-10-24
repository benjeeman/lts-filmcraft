//! Inserting a position inside a move (spec section 3.6). The new position takes the window evaluated at its frame (stored
//! with directed rounding), and both halves keep the original curve with integer `split_of` values. So a frame moves only
//! by the rounding of the stored window and, in a move that zooms, by what rounding its height does to the zoom schedule:
//! at most an eighth of the height's relative rounding times the move's travel (T-M5 checks this bound).
//!
//! Neither function here looks at `locked` (I16). The planner refuses a split into a locked position before it calls them;
//! the edit hook may split a locked position's move at a junction, which keeps its picture (spec section 4.8).

use filmcraft_time::Tick;

use super::canon::clamp_window;
use super::curve::curve_name;
use super::eval::{move_span, window_at};
use super::types::{Arrival, Curve, Origin, Position, Raster, SplitOf, VerticalPlan, Window, Writer};

/// An exact split of the move that contains a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Split {
    /// Where the new position goes in `positions` (the index of the move's arriving position).
    pub index: usize,
    /// The new position's frame start.
    pub at: Tick,
    /// The window evaluated there, stored with directed rounding.
    pub window: Window,
    /// The new position's arrival: `m - s` frames of the original move (a cut when that is 1 frame).
    pub arrival: Arrival,
    /// The arriving position's new arrival: `f_j - m` frames (a cut when that is 1 frame).
    pub next: Arrival,
}

/// The exact split of the move that contains `frame` strictly inside it (`s < m < f_j`), or None when no move does. A
/// locked arriving position splits too: the caller applies I16 (see the module docs).
pub fn split_move(plan: &VerticalPlan, r: &Raster, frame: i64) -> Option<Split> {
    let at = r.rate.tick_of(frame);
    // only the move into the first position after `at` can contain it (I1: positions are sorted by `at`)
    let to = plan.positions.partition_point(|p| p.at <= at);
    let span = move_span(plan, r, to).filter(|m| m.start < at && at < m.end)?;
    let (offset, total) = span.split.map_or((0, span.frames), |split| (split.offset, split.total));
    let before = u32::try_from(frame.saturating_sub(span.start_frame)).ok()?;
    let after = u32::try_from(span.end_frame.saturating_sub(frame)).ok()?;
    let window = clamp_window(window_at(plan, r, at).window, r, plan.output);
    let arrival = piece(before, span.curve, offset, total)?;
    let next = piece(after, span.curve, offset.checked_add(before)?, total)?;
    Some(Split { index: span.to, at, window, arrival, next })
}

/// The arrival of a piece of `frames` frames at `offset` of a `total`-frame move along `curve`: a cut when it is one frame
/// long (a one-frame move renders exactly as a cut). None for an empty piece.
fn piece(frames: u32, curve: Curve, offset: u32, total: u32) -> Option<Arrival> {
    match frames {
        0 => None,
        1 => Some(Arrival::Cut),
        _ => Some(Arrival::Move { move_frames: frames, curve, split_of: Some(SplitOf { offset, total }) }),
    }
}

/// Applies `split` to the plan it was computed on: inserts the new `fill` position with `id` and sets the arriving
/// position's arrival, locked or not. The caller allocates the id, records who wrote it and applies I16.
pub fn insert_split(plan: &mut VerticalPlan, split: &Split, id: u32, origin: Origin, writer: Writer) {
    let index = split.index.min(plan.positions.len());
    plan.positions.insert(index, Position::new(id, split.at, split.window, split.arrival, origin, writer));
    if let Some(arriving) = plan.positions.get_mut(index + 1) {
        arriving.arrival = split.next;
    }
}

/// The label of the move into `plan.positions[index]`: the curve's name for a whole move; "Ease in-out · part 1/2" for a
/// piece of a run of split moves that tiles its original move; "Ease in-out · frames 20–50 of 50" for any other piece;
/// None for a cut, the first position or an index past the end.
pub fn move_label(plan: &VerticalPlan, index: usize) -> Option<String> {
    if index == 0 {
        return None;
    }
    let Arrival::Move { move_frames, curve, split_of } = plan.positions.get(index)?.arrival else { return None };
    let name = curve_name(&curve);
    let Some(split) = split_of else { return Some(name) };
    // the offset and length of the piece arriving at position `i`, when it is a piece of the same original move
    let piece_at = |i: usize| match plan.positions.get(i).map(|p| p.arrival) {
        Some(Arrival::Move { move_frames: frames, curve: c, split_of: Some(s) }) if c == curve && s.total == split.total => Some((s.offset, frames)),
        _ => None,
    };
    // the run around this piece: back while a piece ends where the next one begins, forward while one begins where the
    // previous one ends (a move on the first position has no effect, so the run starts at position 1 at the earliest)
    let (mut start, mut k) = (split.offset, 1);
    for i in (1..index).rev() {
        match piece_at(i) {
            Some((offset, frames)) if offset.checked_add(frames) == Some(start) => (start, k) = (offset, k + 1),
            _ => break,
        }
    }
    let (from, to, total) = (split.offset, split.offset.saturating_add(move_frames), split.total);
    let (mut end, mut n) = (to, k);
    for i in index + 1..plan.positions.len() {
        match piece_at(i) {
            Some((offset, frames)) if offset == end => (end, n) = (offset.saturating_add(frames), n + 1),
            _ => break,
        }
    }
    if start == 0 && end == total {
        return Some(format!("{name} · part {k}/{n}"));
    }
    Some(format!("{name} · frames {from}–{to} of {total}"))
}
