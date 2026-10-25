//! Validation and repair (spec sections 2.5 and 2.8). [`validate`] checks I1-I21 and I23 on every stored datum of a
//! sequence's vertical set (box tracks and overwritten ranges included) and lists each violation; a plan with one is never
//! evaluated (reads answer `invalid_plan`, writes too, except `vertical.repair`, `vertical.delete` and a person's marks,
//! which change no framing: `setLock`, so a locked position repair refuses can be unlocked, `setReview` without an opener,
//! `flagAttention` and `clearAttention`). Repair runs through
//! the planner (`Write::Repair`): it sorts, merges duplicates keeping the first, snaps off-grid frames, clamps windows and
//! rects with directed rounding, shortens or cuts moves that break I5, I6 or I9, resets invalid curves to Ramp 20/20,
//! drops what cannot be repaired and accepts windows made on a frame of another aspect (`frame_changed`), reporting each
//! change; it never changes a locked position (`locked`).

use std::collections::BTreeSet;

use filmcraft_time::{FrameRate, Tick};
use serde::Serialize;
use serde_json::{Value, json};

use super::annotations::{MAX_ANNOTATIONS, valid_kind, valid_track_id};
use super::area::MAX_AREAS;
use super::canon::{clamp_window, on_grid, q};
use super::curve::{DEFAULT_CURVE, check_curve};
use super::derive::Scene;
use super::eval::{centred_largest, limits};
use super::parts::{MAX_FADE, MAX_PARTS};
use super::plan::{Ctx, Draft, MAX_PLANS, Repair, locked, merge_ranges, not_found_plan};
use super::regions::MAX_REGIONS;
use super::types::{
    ActiveArea, Annotation, Arrival, Change, FrameRange, Layout, MAX_COORDINATE, OutputSize, PartAudio, Position, Raster, Refusal, VE_MAX_FRAMES, VerticalPlan,
    VerticalSet, Window,
};
use super::walls::MAX_CANDIDATES;
use crate::{Project, Sequence, validate_frame_size};

/// The most positions, attention marks, overwritten ranges and positions per range a plan holds (I1, I21, I23).
pub(crate) const MAX_POSITIONS: usize = 10_000;
const MAX_MARKS: usize = 2_000;
const MAX_RANGES: usize = 500;
const MAX_RANGE_POSITIONS: usize = 2_000;

/// The most box tracks, and box-track keys in all (I18).
const MAX_TRACKS: usize = 256;
const MAX_KEYS: usize = 50_000;

/// Containment's tolerance (I3), sequence px.
const EPS: f64 = 1e-6;

/// One broken invariant.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Violation {
    /// `"I1"` to `"I23"`.
    pub invariant: &'static str,
    /// The plan it concerns; None for the set's own data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<u32>,
    pub message: String,
    /// The ids, frames and numbers concerned.
    pub detail: serde_json::Value,
}

/// Every violation of the sequence's vertical set, in a deterministic order (the set's data first, then each plan's).
/// Empty for a sequence without a set.
pub fn validate(project: &Project, seq: &Sequence) -> Vec<Violation> {
    let scene = Scene::of(project, seq);
    let Some(set) = scene.set else { return Vec::new() };
    let mut out = set_violations(&scene, set);
    for plan in &set.plans {
        out.extend(plan_only_violations(&scene, plan));
    }
    out
}

/// The violations that keep a plan from being evaluated: the set's own and the plan's.
pub(crate) fn plan_violations(scene: &Scene, plan: &VerticalPlan) -> Vec<Violation> {
    let Some(set) = scene.set else { return Vec::new() };
    let mut out = set_violations(scene, set);
    out.extend(plan_only_violations(scene, plan));
    out
}

/// Collects violations with their invariant, plan and detail.
struct Report {
    plan: Option<u32>,
    out: Vec<Violation>,
}

impl Report {
    fn add(&mut self, invariant: &'static str, message: &str, detail: Value) {
        self.out.push(Violation { invariant, plan: self.plan, message: message.into(), detail });
    }
}

/// Whether a tick is a frame start on `rate`.
fn aligned(rate: FrameRate, t: Tick) -> bool {
    rate.tick_of(rate.frame_at(t)) == t
}

/// Whether a rect `[x, y, w, h]` is finite, on the grid, of positive size and inside the frame of `r`.
fn rect_ok(rect: &[f64; 4], r: &Raster) -> bool {
    let [x, y, w, h] = *rect;
    rect.iter().all(|v| on_grid(*v)) && w > 0.0 && h > 0.0 && x >= 0.0 && y >= 0.0 && x + w <= f64::from(r.width) && y + h <= f64::from(r.height)
}

/// Whether a half-open tick range is frame-aligned, non-empty and inside `[0, end]`.
fn range_ok(rate: FrameRate, from: Tick, to: Tick, end: i64) -> bool {
    let (a, b) = (rate.frame_at(from), rate.frame_at(to));
    aligned(rate, from) && aligned(rate, to) && a >= 0 && a < b && b <= end
}

/// The number of characters of a text.
fn chars(text: &str) -> usize {
    text.chars().count()
}

/// The set's own violations: I10-I12, I13 (delivery), I14, I15 and I17-I19.
pub(crate) fn set_violations(scene: &Scene, set: &VerticalSet) -> Vec<Violation> {
    let r = scene.r;
    let rate = r.rate;
    let end = r.end_frame;
    let mut v = Report { plan: None, out: Vec::new() };
    if scene.seq.settings.par.0 != scene.seq.settings.par.1 {
        let par = scene.seq.settings.par;
        v.add("I12", "the sequence's pixels are not square", json!({"par": [par.0, par.1]}));
    }
    if set.plans.len() > MAX_PLANS {
        v.add("I11", "a set holds at most 16 plans", json!({"count": set.plans.len(), "bound": MAX_PLANS}));
    }
    let mut ids = BTreeSet::new();
    let mut names = BTreeSet::new();
    for plan in &set.plans {
        if !ids.insert(plan.id) {
            v.add("I11", "plan ids are unique", json!({"plan": plan.id}));
        }
        if !names.insert(plan.name.as_str()) {
            v.add("I11", "plan names are unique", json!({"plan": plan.id, "name": plan.name}));
        }
    }
    let walls = &set.walls;
    let inside = |t: Tick| aligned(rate, t) && rate.frame_at(t) > 0 && rate.frame_at(t) < end;
    let stored: Vec<Tick> = walls.stored.iter().map(|w| w.at).collect();
    let candidates: Vec<Tick> = walls.candidates.iter().map(|c| c.at).collect();
    for (what, list) in [("stored", &stored), ("suppressed", &walls.suppressed), ("candidates", &candidates)] {
        let sorted = list.windows(2).all(|w| w.first().map(|a| rate.frame_at(*a)) < w.get(1).map(|b| rate.frame_at(*b)));
        let bad: Vec<i64> = list.iter().filter(|t| !inside(**t)).map(|t| rate.frame_at(*t)).collect();
        if !sorted || !bad.is_empty() {
            v.add("I10", "wall frames are sorted, unique, frame-aligned and inside (0, end)", json!({"list": what, "frames": bad}));
        }
    }
    if walls.candidates.len() > MAX_CANDIDATES || walls.stored.iter().any(|w| w.score.is_some_and(|s| !s.is_finite())) {
        v.add("I10", "at most 20 000 candidates; every score is finite", json!({"candidates": walls.candidates.len()}));
    }
    if walls.candidates.iter().any(|c| !c.score.is_finite()) {
        v.add("I2", "a candidate's score is not finite", json!({}));
    }
    let analysed_ok = walls.analysed.iter().all(|a| range_ok(rate, a.from, a.to, end));
    let merged = walls.analysed.windows(2).all(|w| match w {
        [a, b] => rate.frame_at(a.to) < rate.frame_at(b.from),
        _ => true,
    });
    if !analysed_ok || !merged {
        v.add("I10", "analysed ranges are sorted, merged and inside [0, end]", json!({"ranges": walls.analysed.len()}));
    }
    part_violations(&mut v, set, &r);
    region_violations(&mut v, set, &r);
    annotation_violations(&mut v, set, &r);
    track_violations(&mut v, set, &r);
    area_violations(&mut v, set, &r);
    let d = &set.delivery;
    if chars(&d.series) > 64 || chars(&d.season) > 64 || chars(&d.episode) > 64 {
        v.add("I13", "a delivery field holds at most 64 characters", json!({}));
    }
    v.out
}

/// I14 (and I13 for part text): parts.
fn part_violations(v: &mut Report, set: &VerticalSet, r: &Raster) {
    let rate = r.rate;
    if set.parts.len() > MAX_PARTS {
        v.add("I14", "a set holds at most 500 parts", json!({"count": set.parts.len(), "bound": MAX_PARTS}));
    }
    let mut ids = BTreeSet::new();
    let mut names: BTreeSet<String> = BTreeSet::new();
    for p in &set.parts {
        if !ids.insert(p.id) {
            v.add("I14", "part ids are unique", json!({"part": p.id}));
        }
        if !names.insert(p.name.to_ascii_lowercase()) {
            v.add("I14", "part names are unique, ignoring case", json!({"part": p.id, "name": p.name}));
        }
        if p.name.is_empty() || chars(&p.name) > 64 {
            v.add("I14", "a part's name has 1 to 64 characters", json!({"part": p.id}));
        }
        if !range_ok(rate, p.from, p.to, r.end_frame) {
            v.add("I14", "a part's frames are aligned, from before to, inside [0, end]", json!({"part": p.id}));
        }
        if p.audio.is_some_and(|a| a.fade_in_frames > MAX_FADE || a.fade_out_frames > MAX_FADE) {
            v.add("I14", "a part's fades are 0 to 250 frames", json!({"part": p.id}));
        }
        if chars(&p.meta.title) > 200 || chars(&p.meta.description) > 5_000 {
            v.add("I13", "a part's title and description hold at most 200 and 5 000 characters", json!({"part": p.id}));
        }
    }
    if !set.parts.windows(2).all(|w| w.first().map(|a| a.from) <= w.get(1).map(|b| b.from)) {
        v.add("I14", "parts are sorted by from", json!({}));
    }
}

/// I15 (and I13 for labels): protected regions.
fn region_violations(v: &mut Report, set: &VerticalSet, r: &Raster) {
    if set.regions.len() > MAX_REGIONS {
        v.add("I15", "a set holds at most 2 000 regions", json!({"count": set.regions.len(), "bound": MAX_REGIONS}));
    }
    let mut ids = BTreeSet::new();
    for g in &set.regions {
        if !ids.insert(g.id) {
            v.add("I15", "region ids are unique", json!({"region": g.id}));
        }
        if !rect_ok(&g.rect, r) || !range_ok(r.rate, g.from, g.to, r.end_frame) {
            v.add("I15", "a region's rect lies on the grid inside the frame, its frames from before to", json!({"region": g.id}));
        }
        if chars(&g.label) > 40 {
            v.add("I13", "a region's label holds at most 40 characters", json!({"region": g.id}));
        }
    }
}

/// I17: annotations.
fn annotation_violations(v: &mut Report, set: &VerticalSet, r: &Raster) {
    let rate = r.rate;
    if set.annotations.len() > MAX_ANNOTATIONS {
        v.add("I17", "a set holds at most 20 000 annotations", json!({"count": set.annotations.len(), "bound": MAX_ANNOTATIONS}));
    }
    let mut ids = BTreeSet::new();
    for a in &set.annotations {
        if !ids.insert(a.id) {
            v.add("I17", "annotation ids are unique", json!({"annotation": a.id}));
        }
        let score_ok = a.score.is_none_or(on_grid);
        if !valid_kind(&a.kind) || !range_ok(rate, a.from, a.to, r.end_frame) || chars(&a.label) > 64 || !score_ok {
            v.add("I17", "an annotation's kind, frames, label or score is not valid", json!({"annotation": a.id}));
        }
    }
    let key = |a: &Annotation| (rate.frame_at(a.from), a.id);
    if !set.annotations.windows(2).all(|w| w.first().map(key) <= w.get(1).map(key)) {
        v.add("I17", "annotations are sorted by (from, id)", json!({}));
    }
}

/// I18: box tracks.
fn track_violations(v: &mut Report, set: &VerticalSet, r: &Raster) {
    let rate = r.rate;
    let keys: usize = set.box_tracks.iter().map(|t| t.keys.len()).sum();
    if set.box_tracks.len() > MAX_TRACKS || keys > MAX_KEYS {
        v.add("I18", "at most 256 box tracks and 50 000 keys in all", json!({"tracks": set.box_tracks.len(), "keys": keys}));
    }
    if !set.box_tracks.windows(2).all(|w| w.first().map(|a| &a.id) < w.get(1).map(|b| &b.id)) {
        v.add("I18", "box tracks are sorted by id, unique", json!({}));
    }
    for t in &set.box_tracks {
        if !valid_track_id(&t.id) || chars(&t.label) > 64 || chars(&t.kind) > 32 {
            v.add("I18", "a box track's id, label or kind is not valid", json!({"track": t.id}));
        }
        let sorted = t.keys.windows(2).all(|w| w.first().map(|a| rate.frame_at(a.at)) < w.get(1).map(|b| rate.frame_at(b.at)));
        let inside = t.keys.iter().all(|k| aligned(rate, k.at) && rate.frame_at(k.at) >= 0 && rate.frame_at(k.at) < r.end_frame && rect_ok(&k.rect, r));
        if !sorted || !inside {
            v.add("I18", "a box track's keys are sorted, unique, aligned, inside the frame and on the grid", json!({"track": t.id}));
        }
    }
}

/// I19: active areas.
fn area_violations(v: &mut Report, set: &VerticalSet, r: &Raster) {
    let rate = r.rate;
    if set.active_area.len() > MAX_AREAS {
        v.add("I19", "a set holds at most 2 000 active areas", json!({"count": set.active_area.len(), "bound": MAX_AREAS}));
    }
    let apart = set.active_area.windows(2).all(|w| match w {
        [a, b] => rate.frame_at(a.to) <= rate.frame_at(b.from),
        _ => true,
    });
    let each = set.active_area.iter().all(|a| range_ok(rate, a.from, a.to, r.end_frame) && rect_ok(&a.rect, r));
    if !apart || !each {
        v.add("I19", "active areas are sorted, apart, aligned, inside the frame and on the grid", json!({}));
    }
}

/// The violations of one plan: I1-I9, I11, I13, I20, I21 and I23.
pub(crate) fn plan_only_violations(scene: &Scene, plan: &VerticalPlan) -> Vec<Violation> {
    let r = scene.r;
    let rate = r.rate;
    let mut v = Report { plan: Some(plan.id), out: Vec::new() };
    let out = plan.output;
    let even = out.width >= 2 && out.height >= 2 && out.width.is_multiple_of(2) && out.height.is_multiple_of(2);
    let heights = limits(&r, out);
    if !even || validate_frame_size(out.width, out.height).is_err() || heights.min_height > heights.max_height {
        v.add("I11", "the output's sides are even, at least 2, and need at most 4x upscale", json!({"width": out.width, "height": out.height}));
    }
    if plan.name.is_empty() || chars(&plan.name) > 120 {
        v.add("I13", "a plan's name has 1 to 120 characters", json!({}));
    }
    if let Some(w) = plan.starting_window
        && let Some((inv, why)) = window_problem(&w, &r, out)
    {
        v.add(inv, why, json!({"startingWindow": [w.cx, w.cy, w.h]}));
    }
    position_list(&mut v, &plan.positions, &r, out, Some(&scene.wall_frames));
    let attention_sorted = plan.attention.windows(2).all(|w| w.first().map(|a| rate.frame_at(a.at)) < w.get(1).map(|b| rate.frame_at(b.at)));
    let attention_ok = plan.attention.iter().all(|m| aligned(rate, m.at) && (0..r.end_frame).contains(&rate.frame_at(m.at)) && chars(&m.note) <= 1_000);
    if !attention_sorted || !attention_ok || plan.attention.len() > MAX_MARKS {
        v.add("I21", "attention marks are sorted, unique, aligned, inside [0, end), at most 2 000", json!({}));
    }
    let mut approved = BTreeSet::new();
    for a in &plan.part_approvals {
        if !approved.insert(a.part) || chars(&a.note) > 1_000 {
            v.add("I21", "a plan holds at most one approval per part", json!({"part": a.part}));
        }
    }
    let mut ranges = BTreeSet::new();
    let sorted = plan.overwritten.windows(2).all(|w| w.first().map(|a| a.from) <= w.get(1).map(|b| b.from));
    if !sorted || plan.overwritten.len() > MAX_RANGES {
        v.add("I23", "overwritten ranges are sorted by from, at most 500", json!({}));
    }
    for o in &plan.overwritten {
        if !ranges.insert(o.id) || !range_ok(rate, o.from, o.to, r.end_frame) || o.positions.len() > MAX_RANGE_POSITIONS {
            v.add("I23", "an overwritten range is aligned, from before to, inside [0, end], with a unique id", json!({"range": o.id}));
        }
        if !range_positions_ok(&o.positions, &r, out) {
            v.add("I23", "an overwritten range holds an invalid position", json!({"range": o.id}));
        }
    }
    v.out
}

/// What breaks a stored window: I2 (a number), I4 (its height) or I3 (its rect outside the frame).
fn window_problem(w: &Window, r: &Raster, out: OutputSize) -> Option<(&'static str, &'static str)> {
    let finite = w.cx.is_finite() && w.cy.is_finite() && w.h.is_finite();
    if !finite || w.cx.abs() > MAX_COORDINATE || w.cy.abs() > MAX_COORDINATE || w.h > MAX_COORDINATE {
        return Some(("I2", "a window holds a number that is not finite or beyond 65 536"));
    }
    let heights = limits(r, out);
    if w.h < heights.min_height || w.h > heights.max_height {
        return Some(("I4", "a window's height lies outside the plan's limits"));
    }
    let rect = w.rect(out);
    let inside = rect.x >= -EPS && rect.y >= -EPS && rect.x + rect.w <= f64::from(r.width) + EPS && rect.y + rect.h <= f64::from(r.height) + EPS;
    (!inside).then_some(("I3", "a window lies outside the frame"))
}

/// Whether the positions of an overwritten range are valid on their own (I23): I1, I2-I4, I7, I8, I13 and I20.
fn range_positions_ok(positions: &[Position], r: &Raster, out: OutputSize) -> bool {
    let mut inner = Report { plan: None, out: Vec::new() };
    position_list(&mut inner, positions, r, out, None);
    inner.out.is_empty()
}

/// I1-I9, I13 and I20 over a list of positions. With `walls` the list is a plan's (moves are checked against their room,
/// the walls and the layouts); without, an overwritten range's, each position on its own.
fn position_list(v: &mut Report, positions: &[Position], r: &Raster, out: OutputSize, walls: Option<&[i64]>) {
    let rate = r.rate;
    if positions.len() > MAX_POSITIONS {
        v.add("I1", "a plan holds at most 10 000 positions", json!({"count": positions.len()}));
    }
    let frames: Vec<i64> = positions.iter().map(|p| rate.frame_at(p.at)).collect();
    if !frames.windows(2).all(|w| w.first() < w.get(1)) {
        v.add("I1", "positions are sorted by frame, unique", json!({"frames": frames.len()}));
    }
    let mut ids = BTreeSet::new();
    for (k, p) in positions.iter().enumerate() {
        let f = frames.get(k).copied().unwrap_or(0);
        if !aligned(rate, p.at) || f < 0 || f >= r.end_frame || !ids.insert(p.id) {
            v.add("I1", "a position is frame-aligned, inside [0, end), with a unique id", json!({"position": p.id, "frame": f}));
        }
        if let Some((inv, why)) = window_problem(&p.window, r, out) {
            v.add(inv, why, json!({"position": p.id}));
        }
        metadata(v, p, r);
        let Arrival::Move { move_frames, curve, split_of } = p.arrival else { continue };
        if check_curve(&curve).is_err() {
            v.add("I7", "a move's curve is not valid", json!({"position": p.id}));
        }
        if split_of.is_some_and(|s| s.total == 0 || s.total > VE_MAX_FRAMES || u64::from(s.offset) + u64::from(move_frames) > u64::from(s.total)) {
            v.add("I8", "a split piece lies inside its original move of at most 1 000 000 frames", json!({"position": p.id}));
        }
        if !(2..=VE_MAX_FRAMES).contains(&move_frames) {
            v.add("I5", "a move has 2 to 1 000 000 frames", json!({"position": p.id, "moveFrames": move_frames}));
            continue;
        }
        let (Some(k_before), Some(walls)) = (k.checked_sub(1), walls) else { continue };
        let Some(departing) = positions.get(k_before) else { continue };
        let s = f.saturating_sub(i64::from(move_frames));
        let previous = frames.get(k_before).copied().unwrap_or(0);
        if s < previous {
            v.add("I5", "a move starts no earlier than the position before", json!({"position": p.id, "earliestStart": previous}));
        }
        if departing.layout == Layout::Fit || p.layout == Layout::Fit {
            v.add("I9", "a move never arrives at or departs from a fit position", json!({"position": p.id}));
            continue;
        }
        let inside = walls.partition_point(|&c| c <= s.max(previous)) < walls.partition_point(|&c| c <= f);
        if inside {
            v.add("I6", "a move contains a wall", json!({"position": p.id}));
        }
    }
}

/// I13 and I20: a position's text and metadata.
fn metadata(v: &mut Report, p: &Position, r: &Raster) {
    let tags_ok = p.tags.len() <= 16 && p.tags.iter().all(|t| (1..=64).contains(&chars(t)) && !t.chars().any(char::is_control));
    if chars(&p.label) > 40 || !tags_ok || chars(&p.note) > 1_000 {
        v.add("I13", "a position's label, tags or note is too long", json!({"position": p.id}));
    }
    let subjects_ok = p.subjects.len() <= 8 && p.subjects.iter().all(|s| valid_track_id(s));
    let focus_ok = p.focus.is_none_or(|point| point.iter().all(|v| on_grid(*v)) && focus_inside(point, r));
    let confidence_ok = p.confidence.is_none_or(|c| on_grid(c) && (0.0..=1.0).contains(&c));
    if !subjects_ok || !focus_ok || !confidence_ok || chars(&p.risk) > 200 {
        v.add("I20", "a position's subjects, focus, confidence or risk is not valid", json!({"position": p.id}));
    }
}

/// The frame bounds of focus points: inside `[0, W] × [0, H]`.
fn focus_inside(p: [f64; 2], r: &Raster) -> bool {
    p[0] >= 0.0 && p[1] >= 0.0 && p[0] <= f64::from(r.width) && p[1] <= f64::from(r.height)
}

/// `vertical.repair` (spec section 2.8): the deterministic repair of plan `w.plan` and of the set's data, each change
/// reported in order. Refused `locked {ids}` when a repair step would change or drop a locked position.
pub(crate) fn repair(ctx: &Ctx, draft: &mut Draft, w: &Repair) -> Result<(), Refusal> {
    let r = ctx.r;
    let Some(index) = draft.set.plans.iter().position(|p| p.id == w.plan) else { return Err(not_found_plan(w.plan)) };
    // repair starts from data that breaks invariants: the planner refuses only the violations it would add
    draft.lenient = true;
    let mut changes = Vec::new();
    repair_set(&mut draft.set, &r, &mut changes);
    let walls = Scene::new(ctx.project, ctx.seq, Some(&draft.set)).wall_frames;
    let Some(plan) = draft.set.plans.get_mut(index) else { return Err(not_found_plan(w.plan)) };
    let held = repair_plan(plan, &r, &walls, &mut changes);
    if !held.is_empty() {
        return Err(locked(held));
    }
    draft.changes.extend(changes);
    draft.label = format!("Vertical: repair plan {}", plan.name);
    Ok(())
}

/// One reported repair change.
fn change(changes: &mut Vec<Change>, code: &'static str, detail: Value) {
    changes.push(Change { code, detail });
}

/// Repairs one plan in place. Returns the locked positions a step would change or drop (the caller then refuses).
fn repair_plan(plan: &mut VerticalPlan, r: &Raster, walls: &[i64], changes: &mut Vec<Change>) -> Vec<u32> {
    let rate = r.rate;
    let id = plan.id;
    let before = plan.positions.clone();
    let mut held = Vec::new();
    for p in &mut plan.positions {
        if !aligned(rate, p.at) {
            p.at = rate.tick_of(rate.frame_at(p.at));
            change(changes, "snapped", json!({"plan": id, "id": p.id}));
        }
    }
    let order: Vec<u32> = plan.positions.iter().map(|p| p.id).collect();
    plan.positions.sort_by_key(|p| rate.frame_at(p.at));
    if plan.positions.iter().map(|p| p.id).ne(order.iter().copied()) {
        change(changes, "sorted", json!({"plan": id}));
    }
    let mut kept: Vec<Position> = Vec::with_capacity(plan.positions.len());
    let mut ids = BTreeSet::new();
    for p in std::mem::take(&mut plan.positions) {
        let f = rate.frame_at(p.at);
        let duplicate = kept.last().is_some_and(|k| rate.frame_at(k.at) == f) || ids.contains(&p.id);
        let finite = p.window.cx.is_finite() && p.window.cy.is_finite() && p.window.h.is_finite();
        let reason = if duplicate {
            Some("duplicate")
        } else if f < 0 || f >= r.end_frame {
            Some("outside the sequence")
        } else if !finite || kept.len() >= MAX_POSITIONS {
            Some("unusable")
        } else {
            None
        };
        if let Some(reason) = reason {
            held.extend(p.locked.then_some(p.id));
            change(changes, "dropped", json!({"plan": id, "id": p.id, "reason": reason}));
            continue;
        }
        ids.insert(p.id);
        kept.push(p);
    }
    plan.positions = kept;
    for p in &mut plan.positions {
        let w = clamp_window(p.window, r, plan.output);
        if w != p.window {
            let detail = json!({"plan": id, "id": p.id, "dx": w.cx - p.window.cx, "dy": w.cy - p.window.cy, "dh": w.h - p.window.h});
            change(changes, "clamped", detail);
            p.window = w;
        }
        if trim_metadata(p, r) {
            change(changes, "trimmed", json!({"plan": id, "id": p.id}));
        }
    }
    repair_arrivals(plan, r, walls, changes);
    if let Some(w) = plan.starting_window {
        let usable = if w.cx.is_finite() && w.cy.is_finite() && w.h.is_finite() { w } else { centred_largest(r, plan.output) };
        let fixed = clamp_window(usable, r, plan.output);
        if fixed != w {
            plan.starting_window = Some(fixed);
            change(changes, "clamped", json!({"plan": id, "startingWindow": true}));
        }
    }
    for p in &plan.positions {
        let changed = before.iter().find(|b| b.id == p.id).is_some_and(|b| b != p);
        held.extend((changed && p.locked).then_some(p.id));
    }
    repair_marks(plan, r, changes);
    let ranges = plan.overwritten.len();
    let output = plan.output;
    plan.overwritten.retain(|o| range_ok(rate, o.from, o.to, r.end_frame) && range_positions_ok(&o.positions, r, output));
    if plan.overwritten.len() != ranges {
        change(changes, "dropped", json!({"plan": id, "overwrittenRanges": ranges - plan.overwritten.len()}));
    }
    // the windows, clamped above, now belong to the sequence's frame: `frame_changed` is fixed (spec section 4.8, step 11)
    if let Some(from) = plan.framed_on.take() {
        change(changes, "frame_accepted", json!({"plan": id, "from": from, "to": [r.width, r.height]}));
    }
    held.sort_unstable();
    held.dedup();
    held
}

/// Arrivals: invalid curves to Ramp 20/20, invalid splits to whole moves, moves under 2 frames, across a layout change
/// or without room to cuts, moves longer than their room shortened, moves across a wall started on it.
fn repair_arrivals(plan: &mut VerticalPlan, r: &Raster, walls: &[i64], changes: &mut Vec<Change>) {
    let rate = r.rate;
    let id = plan.id;
    let frames: Vec<i64> = plan.positions.iter().map(|p| rate.frame_at(p.at)).collect();
    let layouts: Vec<Layout> = plan.positions.iter().map(|p| p.layout).collect();
    for (k, p) in plan.positions.iter_mut().enumerate() {
        let Arrival::Move { move_frames, curve, split_of } = p.arrival else { continue };
        let mut curve = curve;
        if check_curve(&curve).is_err() {
            curve = DEFAULT_CURVE;
            change(changes, "curve_reset", json!({"plan": id, "id": p.id}));
        }
        let split_ok = split_of.is_none_or(|s| s.total > 0 && s.total <= VE_MAX_FRAMES && u64::from(s.offset) + u64::from(move_frames) <= u64::from(s.total));
        let mut split = if split_ok { split_of } else { None };
        let f = frames.get(k).copied().unwrap_or(0);
        let mut frames_now = i64::from(move_frames.min(VE_MAX_FRAMES));
        let mut cut_reason = None;
        if frames_now < 2 {
            cut_reason = Some("shorter than 2 frames");
        }
        if let Some(previous) = k.checked_sub(1).and_then(|j| frames.get(j)).copied() {
            let fit = k.checked_sub(1).and_then(|j| layouts.get(j)) == Some(&Layout::Fit) || layouts.get(k) == Some(&Layout::Fit);
            if fit {
                cut_reason = Some("a layout change");
            }
            frames_now = frames_now.min(f - previous);
            let s = f - frames_now;
            let latest = walls.iter().copied().filter(|&c| c > s && c <= f).max();
            if let Some(c) = latest {
                frames_now = f - c;
            }
            if frames_now < 2 && cut_reason.is_none() {
                cut_reason = Some("no room");
            }
        }
        if let Some(reason) = cut_reason {
            p.arrival = Arrival::Cut;
            change(changes, "move_cut", json!({"plan": id, "id": p.id, "reason": reason}));
            continue;
        }
        let frames_u = u32::try_from(frames_now).unwrap_or(move_frames);
        if frames_u != move_frames {
            split = None;
            change(changes, "move_shortened", json!({"plan": id, "id": p.id, "frames": [move_frames, frames_u]}));
        } else if !split_ok {
            change(changes, "split_reset", json!({"plan": id, "id": p.id}));
        }
        p.arrival = Arrival::Move { move_frames: frames_u, curve, split_of: split };
    }
}

/// I13 and I20 on a position: texts cut to their bounds, invalid tags and subjects dropped, the focus and the confidence
/// put on their grids inside their bounds (dropped when not finite). Returns whether anything changed.
fn trim_metadata(p: &mut Position, r: &Raster) -> bool {
    let before = p.clone();
    let cut = |text: &mut String, bound: usize| {
        if chars(text) > bound {
            *text = text.chars().take(bound).collect();
        }
    };
    cut(&mut p.label, 40);
    cut(&mut p.note, 1_000);
    cut(&mut p.risk, 200);
    p.tags.retain(|t| (1..=64).contains(&chars(t)) && !t.chars().any(char::is_control));
    p.tags.truncate(16);
    p.subjects.retain(|s| valid_track_id(s));
    p.subjects.truncate(8);
    let (fw, fh) = (f64::from(r.width), f64::from(r.height));
    p.focus = p.focus.filter(|[x, y]| x.is_finite() && y.is_finite()).map(|[x, y]| [q(x).max(0.0).min(fw), q(y).max(0.0).min(fh)]);
    p.confidence = p.confidence.filter(|c| c.is_finite()).map(|c| q(c.clamp(0.0, 1.0)));
    *p != before
}

/// I21 on attention marks: snapped, sorted, the first of a frame kept, inside `[0, end)`, notes cut, at most 2 000.
fn repair_marks(plan: &mut VerticalPlan, r: &Raster, changes: &mut Vec<Change>) {
    let rate = r.rate;
    let before = plan.attention.clone();
    let mut marks = std::mem::take(&mut plan.attention);
    for m in &mut marks {
        m.at = rate.tick_of(rate.frame_at(m.at));
        if chars(&m.note) > 1_000 {
            m.note = m.note.chars().take(1_000).collect();
        }
    }
    marks.sort_by_key(|m| rate.frame_at(m.at));
    marks.dedup_by_key(|m| rate.frame_at(m.at));
    marks.retain(|m| (0..r.end_frame).contains(&rate.frame_at(m.at)));
    marks.truncate(MAX_MARKS);
    if marks != before {
        change(changes, "marks", json!({"plan": plan.id, "before": before.len(), "after": marks.len()}));
    }
    plan.attention = marks;
    let mut seen = BTreeSet::new();
    let approvals = plan.part_approvals.len();
    plan.part_approvals.retain(|a| seen.insert(a.part));
    if plan.part_approvals.len() != approvals {
        change(changes, "dropped", json!({"plan": plan.id, "approvals": approvals - plan.part_approvals.len()}));
    }
}

/// The set's data repaired in place: walls, parts, regions, annotations, box-track keys and active areas.
fn repair_set(set: &mut VerticalSet, r: &Raster, changes: &mut Vec<Change>) {
    let rate = r.rate;
    let end = r.end_frame;
    let snap = |t: Tick| rate.tick_of(rate.frame_at(t));
    let inside = |t: Tick| rate.frame_at(t) > 0 && rate.frame_at(t) < end;
    let walls_before = set.walls.clone();
    let walls = &mut set.walls;
    walls.stored.iter_mut().for_each(|w| w.at = snap(w.at));
    walls.stored.sort_by_key(|w| rate.frame_at(w.at));
    walls.stored.dedup_by_key(|w| rate.frame_at(w.at));
    walls.stored.retain(|w| inside(w.at));
    walls.stored.iter_mut().filter(|w| w.score.is_some_and(|s| !s.is_finite())).for_each(|w| w.score = None);
    walls.suppressed.iter_mut().for_each(|t| *t = snap(*t));
    walls.suppressed.sort_by_key(|t| rate.frame_at(*t));
    walls.suppressed.dedup_by_key(|t| rate.frame_at(*t));
    walls.suppressed.retain(|t| inside(*t));
    walls.candidates.iter_mut().for_each(|c| c.at = snap(c.at));
    walls.candidates.sort_by_key(|c| rate.frame_at(c.at));
    walls.candidates.dedup_by_key(|c| rate.frame_at(c.at));
    walls.candidates.retain(|c| inside(c.at) && c.score.is_finite());
    walls.candidates.truncate(MAX_CANDIDATES);
    let ranges: Vec<[i64; 2]> = walls.analysed.iter().map(|a| [rate.frame_at(a.from).max(0), rate.frame_at(a.to).min(end)]).filter(|[a, b]| a < b).collect();
    walls.analysed = merge_ranges(ranges).into_iter().map(|[a, b]| FrameRange { from: rate.tick_of(a), to: rate.tick_of(b) }).collect();
    if set.walls != walls_before {
        change(changes, "walls", json!({"repaired": true}));
    }
    repair_parts(set, r, changes);
    repair_regions(set, r, changes);
    repair_annotations(set, r, changes);
    repair_tracks(set, r, changes);
    repair_areas(set, r, changes);
}

/// The frames of a tick range clamped into `[0, end]`; None when nothing is left.
fn clamp_range(rate: FrameRate, from: Tick, to: Tick, end: i64) -> Option<(Tick, Tick)> {
    let (a, b) = (rate.frame_at(from).max(0), rate.frame_at(to).min(end));
    (a < b).then(|| (rate.tick_of(a), rate.tick_of(b)))
}

/// A rect put on the grid and clamped into the frame; None when no area is left.
fn clamp_rect(rect: [f64; 4], r: &Raster) -> Option<[f64; 4]> {
    if !rect.iter().all(|v| v.is_finite()) {
        return None;
    }
    let (fw, fh) = (f64::from(r.width), f64::from(r.height));
    let [x, y, w, h] = rect.map(q);
    let (x0, y0) = (x.max(0.0).min(fw), y.max(0.0).min(fh));
    let (x1, y1) = ((x + w).max(0.0).min(fw), (y + h).max(0.0).min(fh));
    (x1 > x0 && y1 > y0).then(|| [x0, y0, q(x1 - x0), q(y1 - y0)]).filter(|c| rect_ok(c, r))
}

/// Parts: ranges clamped, names cut, fades capped, duplicates (ids, names ignoring case) and empty parts dropped, sorted.
fn repair_parts(set: &mut VerticalSet, r: &Raster, changes: &mut Vec<Change>) {
    let rate = r.rate;
    let before = set.parts.clone();
    let mut ids = BTreeSet::new();
    let mut names = BTreeSet::new();
    let mut kept = Vec::new();
    for mut p in std::mem::take(&mut set.parts) {
        if chars(&p.name) > 64 {
            p.name = p.name.chars().take(64).collect();
        }
        let Some((from, to)) = clamp_range(rate, p.from, p.to, r.end_frame) else { continue };
        if p.name.is_empty() || !ids.insert(p.id) || !names.insert(p.name.to_ascii_lowercase()) || kept.len() >= MAX_PARTS {
            continue;
        }
        (p.from, p.to) = (from, to);
        p.audio = p.audio.map(|a| PartAudio { fade_in_frames: a.fade_in_frames.min(MAX_FADE), fade_out_frames: a.fade_out_frames.min(MAX_FADE) });
        p.meta.title = p.meta.title.chars().take(200).collect();
        p.meta.description = p.meta.description.chars().take(5_000).collect();
        kept.push(p);
    }
    kept.sort_by_key(|p| (rate.frame_at(p.from), p.id));
    if kept != before {
        change(changes, "parts", json!({"before": before.len(), "after": kept.len()}));
    }
    set.parts = kept;
}

/// Regions: rects and ranges clamped, labels cut, duplicate ids and empty regions dropped.
fn repair_regions(set: &mut VerticalSet, r: &Raster, changes: &mut Vec<Change>) {
    let before = set.regions.clone();
    let mut ids = BTreeSet::new();
    let mut kept = Vec::new();
    for mut g in std::mem::take(&mut set.regions) {
        let (Some((from, to)), Some(rect)) = (clamp_range(r.rate, g.from, g.to, r.end_frame), clamp_rect(g.rect, r)) else { continue };
        if !ids.insert(g.id) || kept.len() >= MAX_REGIONS {
            continue;
        }
        (g.from, g.to, g.rect) = (from, to, rect);
        g.label = g.label.chars().take(40).collect();
        kept.push(g);
    }
    if kept != before {
        change(changes, "regions", json!({"before": before.len(), "after": kept.len()}));
    }
    set.regions = kept;
}

/// Annotations: the first of an id kept, invalid kinds and empty ranges dropped, scores on the grid, sorted by (from, id).
fn repair_annotations(set: &mut VerticalSet, r: &Raster, changes: &mut Vec<Change>) {
    let rate = r.rate;
    let before = set.annotations.clone();
    let mut ids = BTreeSet::new();
    let mut kept = Vec::new();
    for mut a in std::mem::take(&mut set.annotations) {
        let first = ids.insert(a.id);
        let Some((from, to)) = clamp_range(rate, a.from, a.to, r.end_frame) else { continue };
        if !first || !valid_kind(&a.kind) || kept.len() >= MAX_ANNOTATIONS {
            continue;
        }
        (a.from, a.to) = (from, to);
        a.label = a.label.chars().take(64).collect();
        // a score that has no place on the grid (not finite, or so large that q overflows or does not round-trip) is dropped
        a.score = a.score.map(q).filter(|s| on_grid(*s));
        kept.push(a);
    }
    kept.sort_by_key(|a| (rate.frame_at(a.from), a.id));
    if kept != before {
        change(changes, "annotations", json!({"before": before.len(), "after": kept.len()}));
    }
    set.annotations = kept;
}

/// Box tracks: invalid and duplicate ids dropped, sorted by id; keys snapped, sorted, the first of a frame kept, inside
/// `[0, end)`, rects clamped (keys left with no area dropped); at most 256 tracks and 50 000 keys.
fn repair_tracks(set: &mut VerticalSet, r: &Raster, changes: &mut Vec<Change>) {
    let rate = r.rate;
    let before = set.box_tracks.clone();
    let mut tracks = std::mem::take(&mut set.box_tracks);
    let mut seen = BTreeSet::new();
    tracks.retain(|t| valid_track_id(&t.id) && seen.insert(t.id.clone()));
    tracks.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    tracks.truncate(MAX_TRACKS);
    let mut budget = MAX_KEYS;
    for t in &mut tracks {
        t.label = t.label.chars().take(64).collect();
        t.kind = t.kind.chars().take(32).collect();
        t.keys.iter_mut().for_each(|k| k.at = rate.tick_of(rate.frame_at(k.at)));
        t.keys.sort_by_key(|k| rate.frame_at(k.at));
        t.keys.dedup_by_key(|k| rate.frame_at(k.at));
        t.keys.retain(|k| (0..r.end_frame).contains(&rate.frame_at(k.at)));
        let mut keys = Vec::with_capacity(t.keys.len());
        for mut k in std::mem::take(&mut t.keys) {
            let Some(rect) = clamp_rect(k.rect, r) else { continue };
            k.rect = rect;
            keys.push(k);
        }
        keys.truncate(budget);
        budget -= keys.len();
        t.keys = keys;
    }
    if tracks != before {
        change(changes, "boxTracks", json!({"before": before.len(), "after": tracks.len()}));
    }
    set.box_tracks = tracks;
}

/// Active areas: ranges and rects clamped, empty ones dropped, sorted by `from`, each overlap trimmed off the later area.
fn repair_areas(set: &mut VerticalSet, r: &Raster, changes: &mut Vec<Change>) {
    let rate = r.rate;
    let before = set.active_area.clone();
    let mut areas: Vec<ActiveArea> = Vec::new();
    for mut a in std::mem::take(&mut set.active_area) {
        let (Some((from, to)), Some(rect)) = (clamp_range(rate, a.from, a.to, r.end_frame), clamp_rect(a.rect, r)) else { continue };
        (a.from, a.to, a.rect) = (from, to, rect);
        areas.push(a);
    }
    areas.sort_by_key(|a| rate.frame_at(a.from));
    let mut kept: Vec<ActiveArea> = Vec::with_capacity(areas.len());
    for mut a in areas {
        if let Some(last) = kept.last()
            && rate.frame_at(a.from) < rate.frame_at(last.to)
        {
            a.from = last.to;
        }
        if rate.frame_at(a.from) < rate.frame_at(a.to) && kept.len() < MAX_AREAS {
            kept.push(a);
        }
    }
    if kept != before {
        change(changes, "activeArea", json!({"before": before.len(), "after": kept.len()}));
    }
    set.active_area = kept;
}

/// Whether two output sizes have the same aspect (cross-multiplied, exact).
pub(crate) fn same_aspect(a: OutputSize, b: OutputSize) -> bool {
    u64::from(a.width) * u64::from(b.height) == u64::from(b.width) * u64::from(a.height)
}
