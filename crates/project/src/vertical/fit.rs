//! `vertical.fit` (spec section 7.4): pure geometry. The window (on the grid) whose centre is the regions' union centre and
//! that contains the union plus a margin at the given size, or at the smallest legal size, clamped inside the picture and
//! the active area; and `eyeTrace`, the window whose x puts a point where another point was on the output frame (the
//! helper behind Match eye-trace, at a cut or a JUMP). The caller chooses the regions and the points; the editor chooses
//! nothing.
//!
//! The function is `fit::fit`, not re-exported at the module root: the M1 tests' support module has a `fit` of its own.

use serde::Serialize;
use serde_json::json;

use super::area::{active_rect, clamp_to_area};
use super::canon::{h_from_zoom, q};
use super::derive::{Scene, governing};
use super::eval::{limits, window_at};
use super::measure::output_point;
use super::plan::{bad_number, invalid_plan, not_found_plan, too_many};
use super::regions::visible_in;
use super::types::{Layout, MAX_COORDINATE, Raster, Refusal, Sample, VerticalPlan, Window};
use super::validate::plan_violations;
use crate::{Project, Sequence};

/// How far the union may reach past the window and still fit, sequence px.
const EPS: f64 = 1e-6;

/// `eyeTrace {fromFrame, fromPoint, toPoint}`: put `to_point` (default: the focus governing the target frame) at the
/// output x that `from_point` (default: the focus governing `from_frame`) had at `from_frame`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EyeTrace {
    pub from_frame: i64,
    pub from_point: Option<[f64; 2]>,
    pub to_point: Option<[f64; 2]>,
}

/// What to fit at `frame`: sequence-space rects (`[x, y, w, h]`), a size (`h` or `zoom`), a margin in output px, or an
/// eye trace (then `regions` may be empty).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FitRequest {
    pub frame: i64,
    pub regions: Vec<[f64; 4]>,
    pub h: Option<f64>,
    pub zoom: Option<f64>,
    pub margin: Option<f64>,
    pub eye_trace: Option<EyeTrace>,
}

/// What limited a fit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LimitedBy {
    /// The clamp moved the centre: the picture's (or the active area's) edge.
    Picture,
    /// The union plus margin needs more than the largest window.
    MaxHeight,
    /// The union plus margin is smaller than the smallest window.
    MinHeight,
}

/// The fitted window and how well it fits.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FitResult {
    pub window: Window,
    /// Whether the union plus margin lies inside the window.
    pub fits: bool,
    /// Each region's visible fraction in the window, in call order.
    pub visible: Vec<f64>,
    pub limited_by: Option<LimitedBy>,
}

/// The most regions one fit takes.
const MAX_REGIONS: usize = 64;

/// Fits a window of plan `plan` at `req.frame` (see the module docs). `not_found {plan}`; `invalid_plan`; `bad_number`
/// for a rect or a number that is not finite or a rect with no area; `out_of_range` for a frame outside the sequence, for
/// `h` with `zoom`, or for neither regions nor an eye trace; `not_found {key: "focus"}` for an eye trace without a point.
pub fn fit(project: &Project, seq: &Sequence, plan: u32, req: &FitRequest) -> Result<FitResult, Refusal> {
    let scene = Scene::of(project, seq);
    let p = scene.plan(plan).ok_or_else(|| not_found_plan(plan))?;
    let violations = plan_violations(&scene, p);
    if !violations.is_empty() {
        return Err(invalid_plan(violations));
    }
    let r = scene.r;
    let in_sequence = |f: i64| (0..r.end_frame).contains(&f);
    if !in_sequence(req.frame) || req.eye_trace.is_some_and(|t| !in_sequence(t.from_frame)) {
        let detail = json!({"key": "frame", "value": req.frame, "end": r.end_frame});
        return Err(Refusal::new("out_of_range", "fit works on a frame of the sequence", detail));
    }
    if req.regions.len() > MAX_REGIONS {
        return Err(too_many("regions", req.regions.len(), MAX_REGIONS));
    }
    for (index, rect) in req.regions.iter().enumerate() {
        if !rect.iter().all(|v| v.is_finite() && v.abs() <= MAX_COORDINATE) || rect[2] <= 0.0 || rect[3] <= 0.0 {
            return Err(Refusal::new("bad_number", "a region is a finite rect with an area", json!({"key": "regions", "index": index})));
        }
    }
    if req.regions.is_empty() && req.eye_trace.is_none() {
        return Err(Refusal::new("out_of_range", "fit needs regions or an eye trace", json!({"key": "regions", "minimum": 1})));
    }
    let margin = req.margin.unwrap_or(0.0);
    if !margin.is_finite() || margin < 0.0 {
        return Err(bad_number("margin"));
    }
    let given = size_of(p, &r, req)?;
    let heights = limits(&r, p.output);
    let area = scene.set.map_or([0.0, 0.0, f64::from(r.width), f64::from(r.height)], |set| active_rect(set, &r, r.rate.tick_of(req.frame)));
    let union = union_of(&req.regions);
    let needed = union.map(|[_, _, w, h]| needed_height(w, h, margin, p));
    let target = match req.eye_trace {
        Some(trace) => eye_target(&scene, p, req, trace, given, union)?,
        None => {
            let [ux, uy, uw, uh] = union.unwrap_or_default();
            let h = given.unwrap_or_else(|| needed.unwrap_or(heights.max_height).max(heights.min_height));
            Window { cx: ux + uw * 0.5, cy: uy + uh * 0.5, h }
        }
    };
    let window = clamp_to_area(target, area, &r, p.output);
    let moved = window.cx != q(target.cx) || window.cy != q(target.cy);
    let wanted = given.or(needed).unwrap_or(window.h);
    let limited_by = if wanted > window.h + EPS {
        Some(LimitedBy::MaxHeight)
    } else if moved {
        Some(LimitedBy::Picture)
    } else if wanted < window.h - EPS {
        Some(LimitedBy::MinHeight)
    } else {
        None
    };
    let shown = Sample { window, layout: Layout::Fill, ..Sample::default() };
    let visible = req.regions.iter().map(|rect| visible_in(&shown, p.output, *rect).1).collect();
    let fits = union.is_none_or(|u| contains(&window, p, u, margin));
    Ok(FitResult { window, fits, visible, limited_by })
}

/// The size a request gives: `h`, or `zoom` as spec section 2.6 converts it. `out_of_range` for both or for a zoom that
/// gives no height, `bad_number` for a value that is not finite.
fn size_of(plan: &VerticalPlan, r: &Raster, req: &FitRequest) -> Result<Option<f64>, Refusal> {
    match (req.h, req.zoom) {
        (Some(_), Some(_)) => Err(Refusal::new("out_of_range", "pass h or zoom, not both", json!({"key": "zoom"}))),
        (Some(h), None) if !h.is_finite() || h <= 0.0 || h > MAX_COORDINATE => Err(bad_number("h")),
        (Some(h), None) => Ok(Some(h)),
        (None, Some(zoom)) if !zoom.is_finite() => Err(bad_number("zoom")),
        (None, Some(zoom)) => {
            let h = h_from_zoom(limits(r, plan.output).max_height, zoom).filter(|h| h.is_finite() && *h <= MAX_COORDINATE);
            h.map(Some).ok_or_else(|| Refusal::new("out_of_range", "the zoom gives no usable height", json!({"key": "zoom", "value": zoom})))
        }
        (None, None) => Ok(None),
    }
}

/// The union `[x, y, w, h]` of rects; None for none.
fn union_of(rects: &[[f64; 4]]) -> Option<[f64; 4]> {
    let first = rects.first()?;
    let (mut x0, mut y0, mut x1, mut y1) = (first[0], first[1], first[0] + first[2], first[1] + first[3]);
    for r in rects.iter().skip(1) {
        (x0, y0) = (x0.min(r[0]), y0.min(r[1]));
        (x1, y1) = (x1.max(r[0] + r[2]), y1.max(r[1] + r[3]));
    }
    Some([x0, y0, x1 - x0, y1 - y0])
}

/// The smallest window height that holds a `w` x `h` union plus `margin` output px on every side.
fn needed_height(w: f64, h: f64, margin: f64, plan: &VerticalPlan) -> f64 {
    let (ow, oh) = (f64::from(plan.output.width), f64::from(plan.output.height));
    let by_height = if oh > 2.0 * margin { h / (1.0 - 2.0 * margin / oh) } else { f64::INFINITY };
    let by_width = if ow > 2.0 * margin { w * oh / (ow - 2.0 * margin) } else { f64::INFINITY };
    by_height.max(by_width)
}

/// Whether the union plus `margin` output px lies inside the window.
fn contains(window: &Window, plan: &VerticalPlan, [x, y, w, h]: [f64; 4], margin: f64) -> bool {
    let m = margin * window.h / f64::from(plan.output.height);
    let rect = window.rect(plan.output);
    x - m >= rect.x - EPS && y - m >= rect.y - EPS && x + w + m <= rect.x + rect.w + EPS && y + h + m <= rect.y + rect.h + EPS
}

/// The focus point governing `frame`, in sequence px.
fn focus_at(plan: &VerticalPlan, r: &Raster, frame: i64) -> Option<[f64; 2]> {
    let id = governing(plan, r, frame)?;
    plan.positions.iter().find(|p| p.id == id)?.focus
}

/// The eye-trace target: at the given size (else the window's at the frame), the x that puts `to_point` where
/// `from_point` was on the output at `from_frame`; the regions' centre y, else the window's.
fn eye_target(scene: &Scene, plan: &VerticalPlan, req: &FitRequest, trace: EyeTrace, given: Option<f64>, union: Option<[f64; 4]>) -> Result<Window, Refusal> {
    let r = scene.r;
    let missing = |frame: i64| Refusal::new("not_found", "no focus point governs the frame; pass the point", json!({"key": "focus", "frame": frame}));
    let from_point = trace.from_point.or_else(|| focus_at(plan, &r, trace.from_frame)).ok_or_else(|| missing(trace.from_frame))?;
    let to_point = trace.to_point.or_else(|| focus_at(plan, &r, req.frame)).ok_or_else(|| missing(req.frame))?;
    if !from_point.iter().chain(&to_point).all(|v| v.is_finite()) {
        return Err(bad_number("eyeTrace"));
    }
    let before = window_at(plan, &r, r.rate.tick_of(trace.from_frame));
    let [ox, _] = output_point(&before, &r, plan.output, from_point);
    let current = window_at(plan, &r, r.rate.tick_of(req.frame)).window;
    let h = given.unwrap_or(current.h);
    let (ow, oh) = (f64::from(plan.output.width), f64::from(plan.output.height));
    let k = h / oh;
    let cx = to_point[0] - ox * k + h * ow / oh * 0.5;
    let cy = union.map_or(current.cy, |[_, y, _, uh]| y + uh * 0.5);
    Ok(Window { cx, cy, h })
}
