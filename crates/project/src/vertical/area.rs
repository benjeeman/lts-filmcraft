//! The active picture area (spec section 4.11): the part of the frame that holds picture over a frame range (inside
//! letterbox or pillarbox bars). Outside every range the whole frame is active. It never changes stored data or the
//! invariants (I3 stays the frame); it constrains the helpers: gestures, Alt+0 and Frame here, `vertical.fit` and
//! `seedShots {mode: "starting"}` stay inside it. This module holds I19, `active_rect`, the helpers' clamp, the warning
//! `window_outside_active` and the writes of `vertical.setActiveArea`.

use std::collections::BTreeMap;

use filmcraft_time::Tick;
use serde_json::json;

use super::canon::{ceil_g, clamp_window, floor_g, q};
use super::derive::{Scene, issue, may_touch};
use super::eval::{limits, window_at};
use super::plan::{Ctx, Draft, MAX_LIST, SetActiveArea, check_len, merge_ranges, too_many};
use super::types::{ActiveArea, Issue, Layout, Level, Origin, OutputSize, Raster, Refusal, VerticalPlan, VerticalSet, Window, Writer};
use crate::{Project, Sequence};

/// The most active areas a set holds (I19).
pub const MAX_AREAS: usize = 2_000;

/// How far a window may reach past the active rect and still count as inside it, sequence px.
const EPS: f64 = 1e-6;

/// The active rect `[x, y, w, h]` at tick `t`: the area whose range holds `t`'s frame, else the whole frame.
pub fn active_rect(set: &VerticalSet, r: &Raster, t: Tick) -> [f64; 4] {
    let f = r.rate.frame_at(t);
    let covering = set.active_area.iter().find(|a| r.rate.frame_at(a.from) <= f && f < r.rate.frame_at(a.to));
    covering.map_or([0.0, 0.0, f64::from(r.width), f64::from(r.height)], |a| a.rect)
}

/// `w` moved inside `area` with the directed rounding of spec section 2.6: `h` first, at most the largest height whose
/// window fits in the area (and the frame's limits), then the centre inside the bounds that height allows in the area;
/// the area's centre when it is too small for the smallest window. The clamp of every helper.
pub fn clamp_to_area(w: Window, area: [f64; 4], r: &Raster, out: OutputSize) -> Window {
    let heights = limits(r, out);
    let (ow, oh) = (f64::from(out.width), f64::from(out.height));
    let [ax, ay, aw, ah] = area;
    let fits = floor_g(ah.min(aw * oh / ow));
    let h = q(w.h).min(heights.max_height).min(fits).max(heights.min_height);
    let width = h * ow / oh;
    let cx = inward(q(w.cx), [ceil_g(ax + width * 0.5), floor_g(ax + aw - width * 0.5)], q(ax + aw * 0.5));
    let cy = inward(q(w.cy), [ceil_g(ay + h * 0.5), floor_g(ay + ah - h * 0.5)], q(ay + ah * 0.5));
    clamp_window(Window { cx, cy, h }, r, out)
}

/// The largest window of `out`'s aspect inside `area`, centred on it, on the grid (Alt+0 and Frame here).
pub fn largest_in_area(area: [f64; 4], r: &Raster, out: OutputSize) -> Window {
    let [ax, ay, aw, ah] = area;
    let h = limits(r, out).max_height;
    clamp_to_area(Window { cx: ax + aw * 0.5, cy: ay + ah * 0.5, h }, area, r, out)
}

/// `v` inside `[lo, hi]`, or `centre` when the bounds cross.
fn inward(v: f64, [lo, hi]: [f64; 2], centre: f64) -> f64 {
    if lo > hi { centre } else { v.max(lo).min(hi) }
}

/// The warnings `window_outside_active {position, runs, area}` of plan `plan`: per governing position, the runs of `fill`
/// frames whose window rect is not inside the active rect within 1e-6 px (the window shows bars).
pub fn active_issues(project: &Project, seq: &Sequence, plan: u32) -> Vec<Issue> {
    let scene = Scene::of(project, seq);
    scene.plan(plan).map(|p| active_issues_in(&scene, p, None)).unwrap_or_default()
}

/// The active-area warnings of a plan in a scene; with `near`, only over the areas whose frames may touch it (a warning
/// lies inside its area's frames, and each frame costs a window). Inside a move the frames count for the position it
/// arrives at.
pub(crate) fn active_issues_in(scene: &Scene, plan: &VerticalPlan, near: Option<&[[i64; 2]]>) -> Vec<Issue> {
    let Some(set) = scene.set else { return Vec::new() };
    let r = scene.r;
    let mut runs: BTreeMap<(usize, u32), Vec<[i64; 2]>> = BTreeMap::new();
    for (index, area) in set.active_area.iter().enumerate() {
        let [ax, ay, aw, ah] = area.rect;
        let (from, to) = (r.rate.frame_at(area.from), r.rate.frame_at(area.to));
        if !may_touch(near, [from, to]) {
            continue;
        }
        for f in from.max(0)..to.min(r.end_frame) {
            let s = window_at(plan, &r, r.rate.tick_of(f));
            let Some(position) = s.next.or(s.position) else { continue };
            let rect = s.window.rect(plan.output);
            let inside = rect.x >= ax - EPS && rect.y >= ay - EPS && rect.x + rect.w <= ax + aw + EPS && rect.y + rect.h <= ay + ah + EPS;
            if s.layout != Layout::Fill || inside {
                continue;
            }
            let list = runs.entry((index, position)).or_default();
            match list.last_mut() {
                Some(run) if run[1] == f => run[1] = f + 1,
                _ => list.push([f, f + 1]),
            }
        }
    }
    let mut out = Vec::new();
    for ((index, position), list) in runs {
        let (Some(area), Some(first), Some(last)) = (set.active_area.get(index), list.first(), list.last()) else { continue };
        let frames = Some([first[0], last[1]]);
        out.push(issue("window_outside_active", Level::Warning, Some(plan.id), frames, json!({"position": position, "runs": list, "area": area.rect})));
    }
    out
}

/// An area's rect checked and put on the grid: `bad_area {index, reason}` for one that is not finite, has no area or
/// lies outside the frame.
fn checked_rect(ctx: &Ctx, rect: [f64; 4], index: usize) -> Result<[f64; 4], Refusal> {
    let bad = |reason: &str| Refusal::new("bad_area", format!("active area {index}: {reason}"), json!({"index": index, "reason": reason}));
    if !rect.iter().all(|v| v.is_finite()) {
        return Err(bad("a number is not finite"));
    }
    let grid = rect.map(q);
    let [x, y, w, h] = grid;
    if w <= 0.0 || h <= 0.0 {
        return Err(bad("the rect has no area"));
    }
    if x < 0.0 || y < 0.0 || x + w > f64::from(ctx.r.width) || y + h > f64::from(ctx.r.height) {
        return Err(bad("the rect lies outside the frame"));
    }
    Ok(grid)
}

/// A range of frames checked: `bad_area {index, reason}` unless `0 <= from < to <= end`.
fn checked_range(ctx: &Ctx, [from, to]: [i64; 2], index: usize) -> Result<(), Refusal> {
    if from >= 0 && from < to && to <= ctx.r.end_frame {
        return Ok(());
    }
    let detail = json!({"index": index, "reason": "range", "fromFrame": from, "toFrame": to});
    Err(Refusal::new("bad_area", format!("active area {index}: its frames lie inside the sequence, from before to"), detail))
}

/// One stored area as frames.
#[derive(Clone, Copy, PartialEq)]
struct Span {
    from: i64,
    to: i64,
    rect: [f64; 4],
    origin: Origin,
    by: Writer,
}

/// `spans` without the frames `[from, to)` (an area across them is split).
fn cleared(spans: Vec<Span>, from: i64, to: i64) -> Vec<Span> {
    let mut out = Vec::with_capacity(spans.len() + 1);
    for s in spans {
        if s.to <= from || s.from >= to {
            out.push(s);
            continue;
        }
        if s.from < from {
            out.push(Span { to: from, ..s });
        }
        if s.to > to {
            out.push(Span { from: to, ..s });
        }
    }
    out
}

/// `vertical.setActiveArea {set, clear}`: `clear` first, then each `set` over its range (an area set over frames that had
/// another replaces it there); adjacent equal areas merge.
pub(crate) fn set_active_area(ctx: &Ctx, draft: &mut Draft, w: &SetActiveArea) -> Result<(), Refusal> {
    check_len("set", w.set.len(), MAX_AREAS)?;
    check_len("clear", w.clear.len(), MAX_LIST)?;
    for (index, range) in w.clear.iter().enumerate() {
        checked_range(ctx, *range, index)?;
    }
    let mut fresh = Vec::with_capacity(w.set.len());
    for (index, a) in w.set.iter().enumerate() {
        checked_range(ctx, [a.from_frame, a.to_frame], index)?;
        fresh.push(Span { from: a.from_frame, to: a.to_frame, rect: checked_rect(ctx, a.rect, index)?, origin: ctx.origin(), by: ctx.caller.writer });
    }
    let rate = ctx.r.rate;
    let as_span = |a: &ActiveArea| Span { from: rate.frame_at(a.from), to: rate.frame_at(a.to), rect: a.rect, origin: a.origin, by: a.modified_by };
    let mut spans: Vec<Span> = draft.set.active_area.iter().map(as_span).collect();
    for range in merge_ranges(w.clear.clone()) {
        spans = cleared(spans, range[0], range[1]);
    }
    for s in fresh {
        spans = cleared(spans, s.from, s.to);
        spans.push(s);
    }
    spans.sort_by_key(|s| s.from);
    let mut merged: Vec<Span> = Vec::with_capacity(spans.len());
    for s in spans {
        match merged.last_mut() {
            Some(last) if last.to == s.from && last.rect == s.rect && last.origin == s.origin && last.by == s.by => last.to = s.to,
            _ => merged.push(s),
        }
    }
    if merged.len() > MAX_AREAS {
        return Err(too_many("activeArea", merged.len(), MAX_AREAS));
    }
    let area = |s: Span| ActiveArea { from: rate.tick_of(s.from), to: rate.tick_of(s.to), rect: s.rect, origin: s.origin, modified_by: s.by };
    draft.set.active_area = merged.into_iter().map(area).collect();
    draft.label = "Vertical: active area".into();
    Ok(())
}
