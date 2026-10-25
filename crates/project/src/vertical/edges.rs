//! Part edges, the model side (spec section 4.6): the raw properties of each part's in-edge (its `from`) and out-edge (its
//! `to`) that come from data, and the edge findings for the thresholds a caller passes. The audio levels, the audio onset
//! and the edge lumas are measured by the engine (M4). The editor ranks nothing and suggests nothing.

use filmcraft_time::{FrameRate, Tick};
use serde::Serialize;
use serde_json::json;

use super::derive::{Scene, issue};
use super::eval::window_at;
use super::measure::cue_frames;
use super::types::{Annotation, Issue, Level, Part, VerticalPlan, VerticalSet, Window};
use super::walls::{WallSource, shown_frame};
use crate::{Project, Sequence};

/// A cue on screen on both sides of an edge (it starts before the edge frame and ends after it), with how many of its
/// frames fall before and after the edge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CueStraddle {
    pub track: u64,
    pub cue: u64,
    pub before: i64,
    pub after: i64,
}

/// The nearest cue start and cue end on each side of an edge, as signed frame distances (negative before the edge, 0 on
/// it); a cue's end frame is the first frame after it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CueDistances {
    pub start_before: Option<i64>,
    pub start_after: Option<i64>,
    pub end_before: Option<i64>,
    pub end_after: Option<i64>,
}

/// The nearest effective wall that is a picture cut (walls whose only source is `part` do not count), as a signed frame
/// distance from the edge, with its sources.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WallDistance {
    pub frame: i64,
    pub distance: i64,
    pub sources: Vec<WallSource>,
}

/// The video transition on an enabled video track that covers an edge frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dissolve {
    pub track: u64,
    pub transition: u64,
    /// Its frames, half-open.
    pub from: i64,
    pub to: i64,
}

/// The nearest edge (`from` or `to`) of an annotation of one kind within the caller's window of an edge.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NearestAnnotation {
    pub kind: String,
    pub annotation: u32,
    pub frame: i64,
    pub distance: i64,
}

/// The data-side properties of one edge.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EdgeSide {
    pub frame: i64,
    pub in_cue: Vec<CueStraddle>,
    pub cue_edges: CueDistances,
    pub wall: Option<WallDistance>,
    pub dissolve: Option<Dissolve>,
    /// The annotations covering the edge frame (`from <= edge < to`).
    pub annotations: Vec<u32>,
    pub nearest: Vec<NearestAnnotation>,
}

/// The edge facts of one part (spec section 2.4's `edge_facts`, model side).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EdgeFacts {
    pub part: u32,
    pub name: String,
    pub from: i64,
    pub to: i64,
    pub duration: i64,
    pub in_edge: EdgeSide,
    pub out_edge: EdgeSide,
    /// The plan's windows on the part's first and last frames, when a plan was named.
    pub first_window: Option<Window>,
    pub last_window: Option<Window>,
}

/// The gap (positive) or the overlap (negative) between two consecutive parts by `from`, in frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartGap {
    pub before: u32,
    pub after: u32,
    pub frames: i64,
}

/// The thresholds of the edge findings the model computes (spec section 4.6); each one passed turns its finding on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EdgeThresholds {
    /// `part_boundary_in_cue`.
    pub edge_in_cue: bool,
    /// `part_boundary_near_cut`: the nearest cut 1 to this many frames from an edge.
    pub near_cut_frames: Option<i64>,
    /// `part_boundary_in_dissolve`.
    pub edge_in_dissolve: bool,
    /// `part_boundary_in_annotation {kind}`: an edge strictly inside an annotation of one of these kinds.
    pub edge_annotation_kinds: Vec<String>,
    /// `part_duration`.
    pub min_part_frames: Option<i64>,
    pub max_part_frames: Option<i64>,
    /// `parts_gap`: more than this many frames between consecutive parts.
    pub max_gap_frames: Option<i64>,
    /// `parts_overlap` when `Some(false)`: consecutive parts may not overlap.
    pub overlap: Option<bool>,
}

/// The edge facts of the requested parts, the gaps and overlaps between consecutive parts, and the findings.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartEdges {
    pub parts: Vec<EdgeFacts>,
    pub gaps: Vec<PartGap>,
    pub findings: Vec<Issue>,
}

/// The edge facts of part `part` of `seq`, with the cues of plan `plan`'s caption tracks (every enabled caption track
/// without a plan) and annotation edges within `window` frames of each edge. None when the part or the plan does not
/// exist.
pub fn edge_facts(project: &Project, seq: &Sequence, plan: Option<u32>, part: u32, window: i64) -> Option<EdgeFacts> {
    let scene = Scene::of(project, seq);
    let plan = match plan {
        Some(id) => Some(scene.plan(id)?),
        None => None,
    };
    let part = scene.set?.parts.iter().find(|p| p.id == part)?;
    Some(facts_of(&scene, plan, part, window))
}

/// The edge facts of `parts` (every part when empty) in `from` order, the gaps and overlaps between consecutive parts, and
/// the findings for `thresholds`.
pub fn part_edges(project: &Project, seq: &Sequence, plan: Option<u32>, parts: &[u32], window: i64, thresholds: &EdgeThresholds) -> PartEdges {
    let scene = Scene::of(project, seq);
    let Some(set) = scene.set else { return PartEdges::default() };
    let plan = match plan {
        Some(id) => match scene.plan(id) {
            Some(p) => Some(p),
            None => return PartEdges::default(),
        },
        None => None,
    };
    let rate = scene.r.rate;
    let mut chosen: Vec<&Part> = set.parts.iter().filter(|p| parts.is_empty() || parts.contains(&p.id)).collect();
    chosen.sort_by_key(|p| (rate.frame_at(p.from), p.id));
    let facts: Vec<EdgeFacts> = chosen.iter().map(|p| facts_of(&scene, plan, p, window)).collect();
    let mut findings = Vec::new();
    for f in &facts {
        findings.extend(edge_findings(set, rate, f, thresholds));
    }
    let mut gaps = Vec::with_capacity(facts.len());
    for pair in facts.windows(2) {
        let [a, b] = pair else { continue };
        let g = gap_of(a, b);
        // the frames between the parts (a gap) or held by both (an overlap)
        let frames = Some([a.to.min(b.from), a.to.max(b.from)]);
        if thresholds.max_gap_frames.is_some_and(|max| g.frames > max) {
            findings.push(issue("parts_gap", Level::Warning, None, frames, json!({"before": g.before, "after": g.after, "frames": g.frames})));
        }
        if thresholds.overlap == Some(false) && g.frames < 0 {
            let detail = json!({"before": g.before, "after": g.after, "frames": g.frames.saturating_neg()});
            findings.push(issue("parts_overlap", Level::Warning, None, frames, detail));
        }
        gaps.push(g);
    }
    PartEdges { parts: facts, gaps, findings }
}

/// The gap (positive) or the overlap (negative) between two parts in `from` order.
fn gap_of(a: &EdgeFacts, b: &EdgeFacts) -> PartGap {
    PartGap { before: a.part, after: b.part, frames: b.from.saturating_sub(a.to) }
}

/// The facts of one part in a scene.
fn facts_of(scene: &Scene, plan: Option<&VerticalPlan>, part: &Part, window: i64) -> EdgeFacts {
    let r = scene.r;
    let (from, to) = (r.rate.frame_at(part.from), r.rate.frame_at(part.to));
    let cues = cue_frames(scene, plan);
    let shown = |p: &VerticalPlan, f: i64| window_at(p, &r, r.rate.tick_of(f)).window;
    EdgeFacts {
        part: part.id,
        name: part.name.clone(),
        from,
        to,
        duration: to.saturating_sub(from),
        in_edge: side_of(scene, &cues, from, window),
        out_edge: side_of(scene, &cues, to, window),
        first_window: plan.map(|p| shown(p, from)),
        last_window: plan.map(|p| shown(p, to.saturating_sub(1))),
    }
}

/// The data-side properties of the edge at frame `e`.
fn side_of(scene: &Scene, cues: &[(i64, i64, u64, u64)], e: i64, window: i64) -> EdgeSide {
    let straddle = |&(first, end, cue, track): &(i64, i64, u64, u64)| CueStraddle { track, cue, before: e.saturating_sub(first), after: end.saturating_sub(e) };
    let in_cue = cues.iter().filter(|q| q.0 < e && q.1 > e).map(straddle);
    let (start_before, start_after) = around(cues.iter().map(|q| q.0), e);
    let (end_before, end_after) = around(cues.iter().map(|q| q.1), e);
    EdgeSide {
        frame: e,
        in_cue: in_cue.collect(),
        cue_edges: CueDistances { start_before, start_after, end_before, end_after },
        wall: nearest_cut(scene, e),
        dissolve: dissolve_at(scene, e),
        annotations: covering(scene, e).map(|a| a.id).collect(),
        nearest: nearest_annotations(scene, e, window),
    }
}

/// The signed distances from `e` to the nearest of `frames` at or before it and to the nearest after it.
fn around(frames: impl Iterator<Item = i64> + Clone, e: i64) -> (Option<i64>, Option<i64>) {
    let before = frames.clone().filter(|&f| f <= e).max().map(|f| f.saturating_sub(e));
    (before, frames.filter(|&f| f > e).min().map(|f| f.saturating_sub(e)))
}

/// The nearest effective wall that is a picture cut (a wall whose only source is `part` is not), the earlier on a tie.
fn nearest_cut(scene: &Scene, e: i64) -> Option<WallDistance> {
    let rate = scene.r.rate;
    let mut best: Option<WallDistance> = None;
    for wall in scene.walls.iter().filter(|w| w.sources.iter().any(|s| *s != WallSource::Part)) {
        let f = rate.frame_at(wall.at);
        let distance = f.saturating_sub(e);
        if best.as_ref().is_none_or(|b| distance.unsigned_abs() < b.distance.unsigned_abs()) {
            best = Some(WallDistance { frame: f, distance, sources: wall.sources.clone() });
        }
    }
    best
}

/// The video transition on an enabled video track whose frames hold `e`.
fn dissolve_at(scene: &Scene, e: i64) -> Option<Dissolve> {
    let rate = scene.r.rate;
    for track in scene.seq.video_tracks.iter().filter(|t| t.enabled) {
        for t in &track.transitions {
            let (from, to) = (rate.frame_at(t.start), shown_frame(rate, Tick(t.start.0.saturating_add(t.duration.0))));
            if from <= e && e < to {
                return Some(Dissolve { track: track.id.0, transition: t.id.0, from, to });
            }
        }
    }
    None
}

/// The annotations whose frames hold `e` (`from <= e < to`).
fn covering<'a>(scene: &'a Scene, e: i64) -> impl Iterator<Item = &'a Annotation> {
    let rate = scene.r.rate;
    scene.set.into_iter().flat_map(|s| &s.annotations).filter(move |a| rate.frame_at(a.from) <= e && e < rate.frame_at(a.to))
}

/// Per kind (sorted by kind), the annotation edge nearest to `e` within `window` frames, the earlier on a tie.
fn nearest_annotations(scene: &Scene, e: i64, window: i64) -> Vec<NearestAnnotation> {
    let rate = scene.r.rate;
    let mut best: Vec<NearestAnnotation> = Vec::new();
    for a in scene.set.iter().flat_map(|s| &s.annotations) {
        for f in [rate.frame_at(a.from), rate.frame_at(a.to)] {
            let distance = f.saturating_sub(e);
            if distance.unsigned_abs() > window.unsigned_abs() {
                continue;
            }
            let candidate = NearestAnnotation { kind: a.kind.clone(), annotation: a.id, frame: f, distance };
            match best.iter_mut().find(|n| n.kind == a.kind) {
                Some(n) if (distance.unsigned_abs(), distance) < (n.distance.unsigned_abs(), n.distance) => *n = candidate,
                Some(_) => {}
                None => best.push(candidate),
            }
        }
    }
    best.sort_by(|x, y| x.kind.as_str().cmp(y.kind.as_str()));
    best
}

/// The edge findings of one part for the thresholds passed (all warnings).
fn edge_findings(set: &VerticalSet, rate: FrameRate, f: &EdgeFacts, th: &EdgeThresholds) -> Vec<Issue> {
    let mut out = Vec::new();
    let at = |e: i64| Some([e, e.saturating_add(1)]);
    for side in [&f.in_edge, &f.out_edge] {
        let e = side.frame;
        if th.edge_in_cue && !side.in_cue.is_empty() {
            let cues: Vec<u64> = side.in_cue.iter().map(|c| c.cue).collect();
            out.push(issue("part_boundary_in_cue", Level::Warning, None, at(e), json!({"part": f.part, "frame": e, "cues": cues})));
        }
        if let (Some(n), Some(wall)) = (th.near_cut_frames, &side.wall)
            && (1..=n.unsigned_abs()).contains(&wall.distance.unsigned_abs())
        {
            let detail = json!({"part": f.part, "frame": e, "wall": wall.frame, "distance": wall.distance});
            out.push(issue("part_boundary_near_cut", Level::Warning, None, at(e), detail));
        }
        if let (true, Some(d)) = (th.edge_in_dissolve, side.dissolve) {
            out.push(issue("part_boundary_in_dissolve", Level::Warning, None, at(e), json!({"part": f.part, "frame": e, "transition": d.transition})));
        }
        let listed = set.annotations.iter().filter(|a| th.edge_annotation_kinds.contains(&a.kind));
        for a in listed.filter(|a| rate.frame_at(a.from) < e && e < rate.frame_at(a.to)) {
            let detail = json!({"part": f.part, "frame": e, "kind": a.kind, "annotation": a.id});
            out.push(issue("part_boundary_in_annotation", Level::Warning, None, at(e), detail));
        }
    }
    let frames = f.duration;
    let short = th.min_part_frames.is_some_and(|min| frames < min);
    let long = th.max_part_frames.is_some_and(|max| frames > max);
    if short || long {
        let detail = json!({"part": f.part, "frames": frames, "minPartFrames": th.min_part_frames, "maxPartFrames": th.max_part_frames});
        out.push(issue("part_duration", Level::Warning, None, Some([f.from, f.to]), detail));
    }
    out
}
