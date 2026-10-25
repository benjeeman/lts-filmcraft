//! Protected regions (spec section 4.7): boxes over frame ranges that must stay in the window (`keepIn`), be all in or
//! all out (`allOrNothing`), or stay out (`keepOut`). They are inputs to checks and never move a window. Per frame a
//! region is `in`, `sliced` or `out` of the window; a `fit` frame shows the whole picture, so everything is `in` there.
//! This module holds those states, their runs, the issues `region_cut`, `region_sliced` and `region_visible`, and the
//! writes of `vertical.setRegions` (content-idempotent adds).

use serde::Serialize;
use serde_json::json;

use super::canon::q;
use super::derive::{Scene, issue, may_touch};
use super::eval::window_at;
use super::plan::{Ctx, Draft, MAX_LIST, SetRegions, alloc, chars_within, check_len, not_found_id, too_many};
use super::types::{Issue, Layout, Level, OutputSize, ProtectedRegion, Raster, Refusal, RegionMode, Sample, VerticalPlan};
use super::walls::counted;
use crate::{Project, Sequence};

/// The most protected regions a set holds (I15).
pub const MAX_REGIONS: usize = 2_000;

/// The longest region label (I13).
const MAX_LABEL: usize = 40;

/// Where a box is, relative to the window shown on a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RegionState {
    /// Entirely inside the window (or a `fit` frame).
    In,
    /// Partly inside.
    Sliced,
    /// Entirely outside.
    Out,
}

/// A run of consecutive frames in one state, half-open, with the smallest visible fraction of the box over the run.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionRun {
    pub from_frame: i64,
    pub to_frame: i64,
    pub state: RegionState,
    pub min_visible: f64,
}

/// The share of `rect` (`[x, y, w, h]`, sequence px) that the window of `plan` shows on `frame`: 1 on a `fit` frame,
/// else the area of the intersection with the window's rect over the rect's area (0 for a rect with no area).
pub fn visible_fraction(plan: &VerticalPlan, r: &Raster, frame: i64, rect: [f64; 4]) -> f64 {
    visible_in(&window_at(plan, r, r.rate.tick_of(frame)), plan.output, rect).1
}

/// The state of `rect` on `frame`: `In` at a visible fraction of 1, `Out` at 0, else `Sliced`.
pub fn region_state(plan: &VerticalPlan, r: &Raster, frame: i64, rect: [f64; 4]) -> RegionState {
    visible_in(&window_at(plan, r, r.rate.tick_of(frame)), plan.output, rect).0
}

/// The runs of `rect`'s states over the frames `[from, to)` of `plan` (inside the sequence), in frame order.
pub fn region_runs(plan: &VerticalPlan, r: &Raster, rect: [f64; 4], from: i64, to: i64) -> Vec<RegionRun> {
    let frames = from.max(0)..to.min(r.end_frame);
    runs_of(frames.map(|f| {
        let (state, fraction) = visible_in(&window_at(plan, r, r.rate.tick_of(f)), plan.output, rect);
        (f, state, fraction)
    }))
}

/// The region issues of plan `plan`: `region_cut {region, runs}` (keepIn: frames sliced or out), `region_sliced
/// {region, runs}` (allOrNothing: frames sliced) and `region_visible {region, runs}` (keepOut: frames in or sliced), each
/// a warning over the region's frames, with only the runs that break its mode.
pub fn region_issues(project: &Project, seq: &Sequence, plan: u32) -> Vec<Issue> {
    let scene = Scene::of(project, seq);
    scene.plan(plan).map(|p| region_issues_in(&scene, p, None)).unwrap_or_default()
}

/// The region issues of a plan in a scene; with `near`, only of the regions whose frames may touch it (a region's warning
/// lies inside its frames, and each frame costs a window).
pub(crate) fn region_issues_in(scene: &Scene, plan: &VerticalPlan, near: Option<&[[i64; 2]]>) -> Vec<Issue> {
    let r = scene.r;
    let mut out = Vec::new();
    for region in scene.set.iter().flat_map(|s| &s.regions) {
        let (from, to) = (r.rate.frame_at(region.from), r.rate.frame_at(region.to));
        if !may_touch(near, [from, to]) {
            continue;
        }
        let runs = region_runs(plan, &r, region.rect, from, to);
        out.extend(mode_warning(plan.id, region.id, region.mode, &runs));
    }
    out
}

/// The warning a region's mode raises over its runs, with only the runs that break the mode; None when none does.
pub(crate) fn mode_warning(plan: u32, region: u32, mode: RegionMode, runs: &[RegionRun]) -> Option<Issue> {
    let (code, bad): (&'static str, fn(RegionState) -> bool) = match mode {
        RegionMode::KeepIn => ("region_cut", |s| s != RegionState::In),
        RegionMode::AllOrNothing => ("region_sliced", |s| s == RegionState::Sliced),
        RegionMode::KeepOut => ("region_visible", |s| s != RegionState::Out),
    };
    let broken: Vec<&RegionRun> = runs.iter().filter(|run| bad(run.state)).collect();
    let frames = [broken.first()?.from_frame, broken.last()?.to_frame];
    Some(issue(code, Level::Warning, Some(plan), Some(frames), json!({"region": region, "runs": broken})))
}

/// The state and the visible fraction of `rect` on a frame showing `s`. A box inside the window (compared edge by edge)
/// is `In` at exactly 1; one with no overlap is `Out` at exactly 0.
pub(crate) fn visible_in(s: &Sample, out: OutputSize, rect: [f64; 4]) -> (RegionState, f64) {
    if s.layout == Layout::Fit {
        return (RegionState::In, 1.0);
    }
    let [x, y, w, h] = rect;
    let usable = [x, y, w, h].iter().all(|v| v.is_finite()) && w > 0.0 && h > 0.0;
    if !usable {
        return (RegionState::Out, 0.0);
    }
    let win = s.window.rect(out);
    let (x1, y1, wx1, wy1) = (x + w, y + h, win.x + win.w, win.y + win.h);
    if x >= win.x && y >= win.y && x1 <= wx1 && y1 <= wy1 {
        return (RegionState::In, 1.0);
    }
    let (ix, iy) = (x1.min(wx1) - x.max(win.x), y1.min(wy1) - y.max(win.y));
    if ix <= 0.0 || iy <= 0.0 {
        return (RegionState::Out, 0.0);
    }
    (RegionState::Sliced, ((ix * iy) / (w * h)).min(1.0))
}

/// Consecutive frames in one state merged into runs, each with its smallest visible fraction.
pub(crate) fn runs_of(states: impl Iterator<Item = (i64, RegionState, f64)>) -> Vec<RegionRun> {
    let mut runs: Vec<RegionRun> = Vec::new();
    for (f, state, fraction) in states {
        match runs.last_mut() {
            Some(run) if run.to_frame == f && run.state == state => {
                run.to_frame = f + 1;
                run.min_visible = run.min_visible.min(fraction);
            }
            _ => runs.push(RegionRun { from_frame: f, to_frame: f + 1, state, min_visible: fraction }),
        }
    }
    runs
}

/// A region's range and rect checked and put on the grid: `out_of_range` for a range outside `[0, end]` or empty,
/// `bad_number` for a rect that is not finite or has no area, `outside_picture` for one outside the frame.
fn checked(ctx: &Ctx, label: &str, range: [i64; 2], rect: [f64; 4], index: usize) -> Result<[f64; 4], Refusal> {
    chars_within("label", label, MAX_LABEL)?;
    let [from, to] = range;
    if from < 0 || from >= to || to > ctx.r.end_frame {
        let detail = json!({"key": "fromFrame", "index": index, "fromFrame": from, "toFrame": to});
        return Err(Refusal::new("out_of_range", "a region's frames lie inside the sequence, from before to", detail));
    }
    if !rect.iter().all(|v| v.is_finite()) {
        return Err(Refusal::new("bad_number", "a region's rect holds a number that is not finite", json!({"key": "rect", "index": index})));
    }
    let grid = rect.map(q);
    let [x, y, w, h] = grid;
    if w <= 0.0 || h <= 0.0 {
        return Err(Refusal::new("bad_number", "a region's rect has no area", json!({"key": "rect", "index": index, "rect": grid})));
    }
    let (fw, fh) = (f64::from(ctx.r.width), f64::from(ctx.r.height));
    if x < 0.0 || y < 0.0 || x + w > fw || y + h > fh {
        return Err(Refusal::new("outside_picture", "a region's rect lies inside the frame", json!({"key": "rect", "index": index, "rect": grid})));
    }
    Ok(grid)
}

/// `vertical.setRegions`: `remove`, then `update`, then `add` (an add identical to an existing region, label, range, rect
/// and mode, returns its id).
pub(crate) fn set_regions(ctx: &Ctx, draft: &mut Draft, w: &SetRegions) -> Result<(), Refusal> {
    check_len("add", w.add.len(), MAX_REGIONS)?;
    check_len("update", w.update.len(), MAX_REGIONS)?;
    check_len("remove", w.remove.len(), MAX_LIST)?;
    let rate = ctx.r.rate;
    let before = draft.set.regions.len();
    draft.set.regions.retain(|r| !w.remove.contains(&r.id));
    let removed = before - draft.set.regions.len();
    for (index, u) in w.update.iter().enumerate() {
        let Some(region) = draft.set.regions.iter().find(|r| r.id == u.region).cloned() else { return Err(not_found_id("region", u.region)) };
        let range = [u.from_frame.unwrap_or(rate.frame_at(region.from)), u.to_frame.unwrap_or(rate.frame_at(region.to))];
        let label = u.label.as_ref().unwrap_or(&region.label).clone();
        let rect = checked(ctx, &label, range, u.rect.unwrap_or(region.rect), index)?;
        let mode = u.mode.unwrap_or(region.mode);
        let changed = ProtectedRegion { label, from: rate.tick_of(range[0]), to: rate.tick_of(range[1]), rect, mode, ..region.clone() };
        if changed != region
            && let Some(slot) = draft.set.regions.iter_mut().find(|r| r.id == u.region)
        {
            *slot = ProtectedRegion { modified_by: ctx.caller.writer, ..changed };
        }
    }
    let mut added = 0;
    for (index, a) in w.add.iter().enumerate() {
        let rect = checked(ctx, &a.label, [a.from_frame, a.to_frame], a.rect, index)?;
        let (from, to) = (rate.tick_of(a.from_frame), rate.tick_of(a.to_frame));
        let same = |r: &&ProtectedRegion| r.label == a.label && r.from == from && r.to == to && r.rect == rect && r.mode == a.mode;
        if let Some(existing) = draft.set.regions.iter().find(same) {
            draft.ids.push(existing.id);
            continue;
        }
        let top = draft.set.regions.iter().map(|r| r.id).max().unwrap_or(0);
        let id = alloc(&mut draft.set.next_region_id, top)?;
        let label = a.label.clone();
        let region = ProtectedRegion { id, label, from, to, rect, mode: a.mode, origin: ctx.origin(), modified_by: ctx.caller.writer };
        draft.set.regions.push(region);
        draft.ids.push(id);
        added += 1;
    }
    if draft.set.regions.len() > MAX_REGIONS {
        return Err(too_many("regions", draft.set.regions.len(), MAX_REGIONS));
    }
    draft.label = counted("regions", added, removed);
    Ok(())
}
