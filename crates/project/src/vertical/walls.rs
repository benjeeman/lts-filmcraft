//! Walls (spec sections 2.3 and 4.1-4.4): the effective walls of a sequence with every source, the shown-frame rule of
//! clip starts, this module's own through-edit predicate, and the wall writes (`vertical.setWalls`,
//! `vertical.applyMeasurement`) with the rules W1-W9.
//!
//! The effective walls are (picture edit points, when the set uses them) ∪ (part boundaries) ∪ (stored walls) − (suppressed
//! frames), each inside `(0, end)`. An edit point is the shown first frame of an enabled media, subclip, nest or multicam
//! clip on an enabled video track, except a plain through edit: `filmcraft-edit` has `is_through_edit`, but it depends on
//! this crate, so [`is_through_edit`] repeats it here (an engine test asserts the two agree) and [`is_shot_change`] adds
//! that a through edit whose pieces differ in Motion or in an enabled geometry effect is a shot change (a punch-in).
//! Graphics, adjustment layers and captions make no wall. Walls never enter evaluation: they constrain writes.

use std::collections::BTreeMap;

use filmcraft_time::{FrameRate, Tick};
use serde::Serialize;
use serde_json::{Value, json};

use super::derive::{Scene, raster};
use super::eval::move_span;
use super::plan::{
    ApplyMeasurement, Ctx, Draft, MAX_LIST, MarkerFilter, MeasuredFrame, OnConflict, SetWalls, WallMove, bad_number, bad_wall, check_len, locked, merge_ranges,
    not_found_frame, too_many,
};
use super::types::{Arrival, Candidate, Change, Flag, FrameRange, Issue, Level, Raster, Refusal, StoredWall, VerticalSet, WallOrigin};
use crate::{EffectInstance, ItemKind, Marker, Project, Sequence, TrackItem};

/// The most candidates a set holds (I10).
pub const MAX_CANDIDATES: usize = 20_000;

/// Media-time slack when comparing the pieces of a cut, as `filmcraft-edit`'s `MEDIA_SLACK`.
const MEDIA_SLACK: u64 = 2;

/// Where an effective wall comes from (spec section 4.1), in the order a wall lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WallSource {
    /// The shown first frame of a picture clip, when the set uses edit points. Recomputed on every read.
    Edit,
    /// A part's `from` or `to`. Recomputed on every read.
    Part,
    Detected,
    Agent,
    Manual,
    Marker,
    Reanchored,
}

impl WallSource {
    /// The source of a stored wall of `origin`.
    pub fn of(origin: WallOrigin) -> Self {
        match origin {
            WallOrigin::Detected => Self::Detected,
            WallOrigin::Agent => Self::Agent,
            WallOrigin::Manual => Self::Manual,
            WallOrigin::Marker => Self::Marker,
            WallOrigin::Reanchored => Self::Reanchored,
        }
    }
}

/// An effective wall: the first frame of a new shot, every source it has (sorted, each once) and the stored wall's score.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Wall {
    /// Frame-aligned.
    pub at: Tick,
    pub sources: Vec<WallSource>,
    /// The frame difference a measurement (or the caller's detector) gave the stored wall.
    pub score: Option<f32>,
}

/// The effective walls of `seq`, sorted by `at` (spec section 4.1). A sequence without a vertical set uses edit points.
pub fn walls(project: &Project, seq: &Sequence) -> Vec<Wall> {
    walls_in(project, seq, seq.vertical.as_deref())
}

/// The effective walls of `seq` with `set` as its vertical data (the planner's working copy, or the stored set).
pub(crate) fn walls_in(project: &Project, seq: &Sequence, set: Option<&VerticalSet>) -> Vec<Wall> {
    let r = raster(seq);
    let inside = |f: i64| f > 0 && f < r.end_frame;
    let mut by_frame: BTreeMap<i64, (Vec<WallSource>, Option<f32>)> = BTreeMap::new();
    if set.is_none_or(|s| s.walls.use_edit_points) {
        for f in edit_frames(project, seq, &r) {
            by_frame.entry(f).or_default().0.push(WallSource::Edit);
        }
    }
    if let Some(set) = set {
        for part in &set.parts {
            for f in [r.rate.frame_at(part.from), r.rate.frame_at(part.to)].into_iter().filter(|&f| inside(f)) {
                by_frame.entry(f).or_default().0.push(WallSource::Part);
            }
        }
        for wall in &set.walls.stored {
            let f = r.rate.frame_at(wall.at);
            if inside(f) {
                let entry = by_frame.entry(f).or_default();
                entry.0.push(WallSource::of(wall.origin));
                entry.1 = entry.1.or(wall.score);
            }
        }
        for t in &set.walls.suppressed {
            by_frame.remove(&r.rate.frame_at(*t));
        }
    }
    let wall = |(f, (mut sources, score)): (i64, (Vec<WallSource>, Option<f32>))| {
        sources.sort_unstable();
        sources.dedup();
        Wall { at: r.rate.tick_of(f), sources, score }
    };
    by_frame.into_iter().map(wall).collect()
}

/// The first frame on `rate` that shows content starting at tick `t`: `frame_at(t - 1) + 1`, the first frame whose start
/// tick is at or after `t` (spec section 2.3; `Track::item_at` takes `start <= t`).
pub fn shown_frame(rate: FrameRate, t: Tick) -> i64 {
    rate.frame_at(Tick(t.0.saturating_sub(1))).saturating_add(1)
}

/// Whether the cut between adjacent items `a` (left) and `b` (right) is a through edit: `filmcraft-edit`'s
/// `is_through_edit` (`crates/edit/src/through.rs:21-32`) to the letter: `a` ends where `b` starts, same item, speed,
/// direction, multicam selection and frame hold, and `b` continues `a` in media time within 2 ticks.
pub fn is_through_edit(a: &TrackItem, b: &TrackItem) -> bool {
    let a_end = Tick(a.start.0.saturating_add(a.duration.0));
    if a_end != b.start || a.item != b.item || a.speed != b.speed || a.reverse != b.reverse || a.multicam != b.multicam {
        return false;
    }
    match (a.frame_hold, b.frame_hold) {
        (None, None) => {}
        (Some(x), Some(y)) => return x == y,
        _ => return false,
    }
    let diff = if a.reverse { source_out(b).saturating_sub(a.source_in.0) } else { b.source_in.0.saturating_sub(source_out(a)) };
    diff.unsigned_abs() <= MEDIA_SLACK
}

/// Whether two pieces differ in Motion (its parameters or keyframes) or in their enabled geometry effects (the Transform
/// and Distort categories, Crop, Auto Reframe), compared by their canonical JSON so that "auto" (NaN) points are equal.
/// Scale to Frame Size, which also changes the picture's geometry, counts too.
pub fn geometry_differs(a: &TrackItem, b: &TrackItem) -> bool {
    a.scale_to_frame != b.scale_to_frame || geometry_of(a) != geometry_of(b)
}

/// Whether the start of `b`, which follows `a` on its track, is a shot change: not a through edit, or a through edit
/// whose pieces differ in geometry ([`geometry_differs`]).
pub fn is_shot_change(a: &TrackItem, b: &TrackItem) -> bool {
    !is_through_edit(a, b) || geometry_differs(a, b)
}

/// Picture edit points per minute of `seq` (its `edit` walls over its length), for "shots known" (spec section 4.4).
pub fn edit_density(project: &Project, seq: &Sequence) -> f64 {
    let r = raster(seq);
    if r.end_frame <= 0 {
        return 0.0;
    }
    let rate = r.rate.sane();
    let edits = edit_frames(project, seq, &r).len() as f64;
    edits * 60.0 * rate.num as f64 / (rate.den as f64 * r.end_frame as f64)
}

/// The frames of `seq` where shots are not known, merged and half-open (the info issue `shots_unknown {ranges}`): every
/// frame outside the analysed ranges when the sequence has fewer than `edited_density` edit points per minute, else none.
/// A measurement job running is the engine's to add.
pub fn unknown_ranges(project: &Project, seq: &Sequence, edited_density: f64) -> Vec<[i64; 2]> {
    let r = raster(seq);
    if r.end_frame <= 0 || edit_density(project, seq) >= edited_density {
        return Vec::new();
    }
    let mut analysed: Vec<[i64; 2]> = seq.vertical.iter().flat_map(|s| &s.walls.analysed).map(|a| [r.rate.frame_at(a.from), r.rate.frame_at(a.to)]).collect();
    analysed.sort_unstable();
    let mut out = Vec::new();
    let mut cursor = 0;
    for [from, to] in analysed {
        if from > cursor {
            out.push([cursor, from.min(r.end_frame)]);
        }
        cursor = cursor.max(to);
    }
    if cursor < r.end_frame {
        out.push([cursor, r.end_frame]);
    }
    out.retain(|[a, b]| a < b);
    out
}

/// The shown first frames of the picture clips of `seq` inside `(0, end)` that are shot changes, sorted and unique.
pub(crate) fn edit_frames(project: &Project, seq: &Sequence, r: &Raster) -> Vec<i64> {
    let mut frames = Vec::new();
    for track in seq.video_tracks.iter().filter(|t| t.enabled) {
        let mut items: Vec<&TrackItem> = track.items.iter().collect();
        items.sort_by_key(|i| i.start);
        let mut prev: Option<&TrackItem> = None;
        for item in items {
            let shows = item.enabled && shows_picture(project, item);
            if shows && prev.is_none_or(|p| is_shot_change(p, item)) {
                let f = shown_frame(r.rate, item.start);
                if f > 0 && f < r.end_frame {
                    frames.push(f);
                }
            }
            prev = shows.then_some(item);
        }
    }
    frames.sort_unstable();
    frames.dedup();
    frames
}

/// Whether a track item shows picture that can change at a cut: a media clip with video, a subclip of one, a nest or a
/// multicam clip. Graphics and adjustment layers do not.
pub(crate) fn shows_picture(project: &Project, item: &TrackItem) -> bool {
    match project.item(item.item).map(|i| &i.kind) {
        Some(ItemKind::Media(m)) => m.info.has_video(),
        Some(ItemKind::Sequence(_)) => true,
        Some(ItemKind::Subclip { parent, .. }) => project.item(*parent).is_some_and(|p| p.has_video()),
        _ => false,
    }
}

/// The media time an item consumes, saturating (the source out point of `TrackItem::source_out`).
fn source_out(item: &TrackItem) -> i64 {
    let consumed = (item.duration.0 as f64 * item.speed.abs()).round() as i64;
    item.source_in.0.saturating_add(consumed)
}

/// Motion and the enabled geometry effects of an item, as canonical JSON, in order.
fn geometry_of(item: &TrackItem) -> Vec<Value> {
    let geometric = |e: &&EffectInstance| e.effect == "motion" || (e.enabled && is_geometry(e));
    item.effects.iter().filter(geometric).map(|e| serde_json::to_value(e).unwrap_or(Value::Null)).collect()
}

/// Whether an effect changes the picture's geometry: the Transform and Distort categories, Crop and Auto Reframe.
pub(crate) fn is_geometry(e: &EffectInstance) -> bool {
    e.effect == "crop" || e.effect == "auto_reframe" || e.def().is_some_and(|d| matches!(d.category, ["Video Effects", "Transform" | "Distort"]))
}

/// The earliest and the latest of the sorted wall frames `walls` inside `(s, f]`, if any (W1, W5).
fn walls_inside(walls: &[i64], s: i64, f: i64) -> Option<(i64, i64)> {
    let lo = walls.partition_point(|&c| c <= s);
    let hi = walls.partition_point(|&c| c <= f);
    let earliest = *walls.get(lo)?;
    let latest = *walls.get(hi.checked_sub(1)?)?;
    (lo < hi).then_some((earliest, latest))
}

/// W5 over every plan of the draft (spec section 4.3): a move containing an effective wall `c` (`s < c <= f`) is refused
/// `would_split_move {plan, position, wall}` with `Refuse`; with `Shorten` it starts at the latest such wall (a cut when
/// fewer than 2 frames are left), gains the flag `MoveShortened` (its other flags stay) and is reported `move_shortened`.
/// A locked move is never shortened: `locked {ids}`.
pub(crate) fn split_moves(ctx: &Ctx, draft: &mut Draft, on_conflict: OnConflict) -> Result<(), Refusal> {
    let walls = Scene::new(ctx.project, ctx.seq, Some(&draft.set)).wall_frames;
    let r = ctx.r;
    let mut shorten: Vec<(usize, usize, i64)> = Vec::new();
    let mut held = Vec::new();
    for (pi, plan) in draft.set.plans.iter().enumerate() {
        for k in 1..plan.positions.len() {
            let Some(span) = move_span(plan, &r, k) else { continue };
            let Some((earliest, latest)) = walls_inside(&walls, span.start_frame, span.end_frame) else { continue };
            let Some(p) = plan.positions.get(k) else { continue };
            if on_conflict == OnConflict::Refuse {
                let detail = json!({"plan": plan.id, "position": p.id, "wall": earliest});
                return Err(Refusal::new("would_split_move", "a new wall would fall inside a move; pass onConflict shorten", detail));
            }
            if p.locked {
                held.push(p.id);
            }
            shorten.push((pi, k, latest));
        }
    }
    if !held.is_empty() {
        return Err(locked(held));
    }
    for (pi, k, wall) in shorten {
        let Some(plan) = draft.set.plans.get_mut(pi) else { continue };
        let plan_id = plan.id;
        let Some(p) = plan.positions.get_mut(k) else { continue };
        let Arrival::Move { move_frames, curve, .. } = p.arrival else { continue };
        let left = r.rate.frame_at(p.at).saturating_sub(wall);
        let frames = u32::try_from(left).unwrap_or(0);
        p.arrival = if frames >= 2 { Arrival::Move { move_frames: frames, curve, split_of: None } } else { Arrival::Cut };
        if !p.flags.contains(&Flag::MoveShortened) {
            p.flags.push(Flag::MoveShortened);
        }
        let new = if frames >= 2 { frames } else { 0 };
        draft.flagged.push((plan_id, p.id));
        let detail = json!({"plan": plan_id, "position": p.id, "frames": [move_frames, new], "wall": wall});
        draft.changes.push(Change { code: "move_shortened", detail });
    }
    Ok(())
}

/// `vertical.setWalls` (spec sections 4.1 and 4.3), applied in the order of [`SetWalls`], then W5.
pub(crate) fn set_walls(ctx: &Ctx, draft: &mut Draft, w: &SetWalls) -> Result<(), Refusal> {
    for (key, len) in [("add", w.add.len()), ("remove", w.remove.len()), ("move", w.moves.len()), ("promote", w.promote.len())] {
        check_len(key, len, MAX_LIST)?;
    }
    for (key, len) in [("suppress", w.suppress.len()), ("unsuppress", w.unsuppress.len()), ("analysed", w.analysed.len())] {
        check_len(key, len, MAX_LIST)?;
    }
    if let Some(c) = &w.candidates {
        check_len("candidates", c.set.len(), MAX_CANDIDATES)?;
    }
    let frames = w.remove.iter().chain(&w.suppress).chain(&w.unsuppress).chain(&w.promote).copied();
    for f in frames.chain(w.add.iter().map(|a| a.frame)).chain(w.moves.iter().flat_map(|m| [m.from, m.to])) {
        wall_frame(ctx, f)?;
    }
    for a in &w.add {
        if let Some(s) = a.score {
            score_of(s)?;
        }
    }
    let before = Scene::new(ctx.project, ctx.seq, Some(&draft.set)).wall_frames;
    let rate = ctx.r.rate;
    if let Some(on) = w.use_edit_points {
        draft.set.walls.use_edit_points = on;
    }
    for &f in &w.remove {
        remove_wall(ctx, draft, f)?;
    }
    let origin = ctx.wall_origin();
    let walls = &mut draft.set.walls;
    walls.suppressed.retain(|t| !w.unsuppress.contains(&rate.frame_at(*t)));
    for &f in &w.suppress {
        insert_tick(&mut walls.suppressed, rate.tick_of(f), rate);
    }
    for a in &w.add {
        let score = a.score.map(score_of).transpose()?;
        let at = rate.tick_of(a.frame);
        walls.candidates.retain(|c| c.at != at);
        // an added wall is a wall: a suppression of its frame is lifted, and a stored wall the suppression hid (a detected
        // wall a person deleted) comes back as the caller's, so a new measurement keeps it
        let hidden = walls.suppressed.iter().any(|t| rate.frame_at(*t) == a.frame);
        walls.suppressed.retain(|t| rate.frame_at(*t) != a.frame);
        match walls.stored.iter_mut().find(|s| s.at == at) {
            Some(wall) if hidden => {
                wall.origin = origin;
                wall.score = score.or(wall.score);
            }
            Some(_) => {}
            None => walls.stored.push(StoredWall { at, origin, score }),
        }
    }
    for &f in &w.promote {
        let at = rate.tick_of(f);
        match walls.candidates.iter().position(|c| c.at == at) {
            Some(i) => {
                let score = walls.candidates.get(i).map(|c| c.score);
                walls.candidates.remove(i);
                walls.stored.retain(|s| s.at != at);
                walls.stored.push(StoredWall { at, origin, score });
                // a promoted candidate is a wall, as an added one is
                walls.suppressed.retain(|t| rate.frame_at(*t) != f);
            }
            None if walls.stored.iter().any(|s| s.at == at) => {}
            None => return Err(not_found_frame("candidate", f)),
        }
    }
    sort_walls(&mut draft.set, rate);
    if let Some(filter) = &w.from_markers {
        for f in marker_frames(ctx, filter) {
            let at = rate.tick_of(f);
            if !draft.set.walls.stored.iter().any(|s| s.at == at) {
                draft.set.walls.stored.push(StoredWall { at, origin: WallOrigin::Marker, score: None });
            }
        }
        sort_walls(&mut draft.set, rate);
    }
    let mut carried = 0;
    for m in &w.moves {
        carried += move_wall(ctx, draft, *m)?;
    }
    if let Some(c) = &w.candidates {
        set_candidates(ctx, draft, c.from_frame, c.to_frame, &c.set.iter().map(|c| MeasuredFrame { frame: c.frame, score: c.score }).collect::<Vec<_>>())?;
    }
    for &[from, to] in &w.analysed {
        if from < 0 || from >= to || to > ctx.r.end_frame {
            return Err(Refusal::new("bad_wall", "an analysed range lies inside [0, end] with from < to", json!({"range": [from, to]})));
        }
        add_analysed(&mut draft.set, rate, from, to);
    }
    split_moves(ctx, draft, w.on_conflict)?;
    let after = Scene::new(ctx.project, ctx.seq, Some(&draft.set)).wall_frames;
    let added = after.iter().filter(|f| before.binary_search(f).is_err()).count();
    let removed = before.iter().filter(|f| after.binary_search(f).is_err()).count();
    draft.label = match w.moves.as_slice() {
        [m] if only_moves(w) => {
            let positions = if carried == 0 { String::new() } else { format!(" (+{carried} position{})", plural(carried)) };
            format!("Vertical: move wall {} \u{2192} {}{positions}", ctx.tc(m.from), ctx.tc(m.to))
        }
        _ => counted("walls", added, removed),
    };
    Ok(())
}

/// Whether a setWalls call does nothing but its moves.
fn only_moves(w: &SetWalls) -> bool {
    let lists = [w.add.len(), w.remove.len(), w.promote.len(), w.suppress.len(), w.unsuppress.len(), w.analysed.len()];
    lists.iter().all(|&n| n == 0) && w.candidates.is_none() && w.use_edit_points.is_none() && w.from_markers.is_none()
}

/// The shown frames inside `(0, end)` of the sequence markers that `filter` names (every given key must match).
fn marker_frames(ctx: &Ctx, filter: &MarkerFilter) -> Vec<i64> {
    let rate = ctx.r.rate;
    let named = |m: &&Marker| filter.name.as_ref().is_none_or(|n| *n == m.name) && filter.color.is_none_or(|c| c == m.color);
    let kept = ctx.seq.markers.iter().filter(named).filter(|m| filter.kind.is_none_or(|k| k == m.kind));
    kept.map(|m| shown_frame(rate, m.start)).filter(|&f| f > 0 && f < ctx.r.end_frame).collect()
}

/// The undo label of a write that adds and removes items of one kind: "Vertical: walls (+3 −1)", or "Vertical: walls"
/// when it added and removed none.
pub(crate) fn counted(noun: &str, added: usize, removed: usize) -> String {
    let parts: Vec<String> = [(added > 0).then(|| format!("+{added}")), (removed > 0).then(|| format!("\u{2212}{removed}"))].into_iter().flatten().collect();
    if parts.is_empty() { format!("Vertical: {noun}") } else { format!("Vertical: {noun} ({})", parts.join(" ")) }
}

/// "s" for a count other than one.
pub(crate) fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// A wall frame lies inside `(0, end)`: `bad_wall {frame}` otherwise.
fn wall_frame(ctx: &Ctx, frame: i64) -> Result<(), Refusal> {
    if frame > 0 && frame < ctx.r.end_frame {
        return Ok(());
    }
    Err(bad_wall(frame, "a wall lies inside (0, end)"))
}

/// A caller's score as stored (f32): `bad_number {key: "score"}` when it is not finite there.
fn score_of(score: f64) -> Result<f32, Refusal> {
    let stored = score as f32;
    if score.is_finite() && stored.is_finite() {
        return Ok(stored);
    }
    Err(bad_number("score"))
}

/// Inserts `t` into a sorted, unique tick list.
fn insert_tick(list: &mut Vec<Tick>, t: Tick, rate: FrameRate) {
    let f = rate.frame_at(t);
    if let Err(i) = list.binary_search_by_key(&f, |x| rate.frame_at(*x)) {
        list.insert(i, t);
    }
}

/// Sorts the stored walls and candidates by frame.
fn sort_walls(set: &mut VerticalSet, rate: FrameRate) {
    set.walls.stored.sort_by_key(|w| rate.frame_at(w.at));
    set.walls.candidates.sort_by_key(|c| rate.frame_at(c.at));
}

/// `remove`: a detected wall is suppressed (it stays stored, so a new measurement does not bring it back); a manual,
/// agent, marker or reanchored one is removed. A frame with no stored wall is a no-op, unless an edit or part wall stands
/// there: it can only be suppressed (`bad_wall`).
fn remove_wall(ctx: &Ctx, draft: &mut Draft, frame: i64) -> Result<(), Refusal> {
    let rate = ctx.r.rate;
    let at = rate.tick_of(frame);
    match draft.set.walls.stored.iter().find(|w| w.at == at).map(|w| w.origin) {
        Some(WallOrigin::Detected) => insert_tick(&mut draft.set.walls.suppressed, at, rate),
        Some(_) => draft.set.walls.stored.retain(|w| w.at != at),
        None => {
            let scene = Scene::new(ctx.project, ctx.seq, Some(&draft.set));
            if scene.is_wall(frame) {
                return Err(bad_wall(frame, "an edit or part wall can only be suppressed"));
            }
        }
    }
    Ok(())
}

/// W8: moves the stored wall at `m.from` to `m.to`. The positions on it move with it, a move starting on it starts at the
/// new frame, and the wall takes the mover's origin and keeps its score. A stored wall on a suppressed frame (a detected
/// wall a person deleted) is no wall and does not move, and no wall moves onto a suppressed frame. Returns how many
/// positions it carried.
fn move_wall(ctx: &Ctx, draft: &mut Draft, m: WallMove) -> Result<usize, Refusal> {
    let (from, to) = (m.from, m.to);
    if from == to {
        return Ok(0);
    }
    let rate = ctx.r.rate;
    let origin = ctx.wall_origin();
    let (at_from, at_to) = (rate.tick_of(from), rate.tick_of(to));
    let scene = Scene::new(ctx.project, ctx.seq, Some(&draft.set));
    let suppressed = |f: i64| draft.set.walls.suppressed.iter().any(|t| rate.frame_at(*t) == f);
    let found = draft.set.walls.stored.iter().position(|w| w.at == at_from).filter(|_| !suppressed(from));
    let Some(index) = found else {
        let retried = draft.set.walls.stored.iter().any(|w| w.at == at_to && w.origin == origin);
        if retried && !scene.is_wall(from) {
            return Ok(0);
        }
        if scene.is_wall(from) {
            return Err(bad_wall(from, "only a stored wall moves; an edit point is the edit itself"));
        }
        return Err(not_found_frame("frame", from));
    };
    if draft.set.walls.stored.iter().any(|w| w.at == at_to) {
        return Err(bad_wall(to, "a stored wall already stands on that frame"));
    }
    if suppressed(to) {
        return Err(bad_wall(to, "the frame is suppressed; unsuppress it first"));
    }
    let (lo, hi) = (from.min(to), from.max(to));
    let mut blocked: Vec<i64> = scene.wall_frames.iter().copied().filter(|&c| c > lo && c < hi).collect();
    for plan in &draft.set.plans {
        blocked.extend(plan.positions.iter().map(|p| rate.frame_at(p.at)).filter(|&f| f > lo && f < hi));
        // a position carried onto a frame that holds another (I1)
        let holds = |t: Tick| plan.positions.iter().any(|p| p.at == t);
        if holds(at_from) && holds(at_to) {
            blocked.push(to);
        }
    }
    if !blocked.is_empty() {
        blocked.sort_unstable();
        blocked.dedup();
        return Err(Refusal::new("wall_blocked", "a position or another wall lies between the old and the new frame", json!({"frames": blocked})));
    }
    let r = ctx.r;
    let mut held = Vec::new();
    let mut edits: Vec<(usize, usize, Option<(u32, u32)>)> = Vec::new();
    for (pi, plan) in draft.set.plans.iter().enumerate() {
        for (k, p) in plan.positions.iter().enumerate() {
            let carried = p.at == at_from;
            let start = move_span(plan, &r, k).filter(|s| s.start_frame == from).map(|s| s.frames);
            let Some(old) = start else {
                if carried {
                    held.extend(p.locked.then_some(p.id));
                    edits.push((pi, k, None));
                }
                continue;
            };
            let left = rate.frame_at(p.at).saturating_sub(to);
            let new = u32::try_from(left).unwrap_or(0);
            if new < 2 {
                let detail = json!({"position": p.id, "frames": left, "minimum": 2});
                return Err(Refusal::new("move_too_short", "the move would be shorter than 2 frames", detail));
            }
            held.extend(p.locked.then_some(p.id));
            edits.push((pi, k, Some((old, new))));
        }
    }
    if !held.is_empty() {
        held.sort_unstable();
        held.dedup();
        return Err(locked(held));
    }
    let mut carried_total = 0;
    for plan in &mut draft.set.plans {
        let plan_id = plan.id;
        let mut carried = Vec::new();
        for p in &mut plan.positions {
            if p.at == at_from {
                p.at = at_to;
                carried.push(p.id);
            }
        }
        if !carried.is_empty() {
            carried_total += carried.len();
            draft.changes.push(Change { code: "positions_carried", detail: json!({"plan": plan_id, "positions": carried, "from": from, "to": to}) });
        }
    }
    for (pi, k, lengths) in edits {
        let Some((old, new)) = lengths else { continue };
        let Some(plan) = draft.set.plans.get_mut(pi) else { continue };
        let plan_id = plan.id;
        let Some(p) = plan.positions.get_mut(k) else { continue };
        if let Arrival::Move { curve, .. } = p.arrival {
            p.arrival = Arrival::Move { move_frames: new, curve, split_of: None };
            draft.changes.push(Change { code: "move_start_moved", detail: json!({"plan": plan_id, "position": p.id, "frames": [old, new]}) });
        }
    }
    if let Some(wall) = draft.set.walls.stored.get_mut(index) {
        wall.at = at_to;
        wall.origin = origin;
    }
    sort_walls(&mut draft.set, rate);
    Ok(carried_total)
}

/// Replaces the candidates inside `[from, to)` by `set` (each frame inside that range and inside `(0, end)`, its score
/// finite). Frames that hold a stored wall are skipped.
fn set_candidates(ctx: &Ctx, draft: &mut Draft, from: i64, to: i64, set: &[MeasuredFrame]) -> Result<(), Refusal> {
    let end = ctx.r.end_frame;
    if from < 0 || from >= to || to > end {
        return Err(Refusal::new("bad_wall", "a candidate range lies inside [0, end] with from < to", json!({"range": [from, to]})));
    }
    let rate = ctx.r.rate;
    let mut fresh = Vec::with_capacity(set.len());
    for c in set {
        wall_frame(ctx, c.frame)?;
        if c.frame < from || c.frame >= to {
            return Err(bad_wall(c.frame, "a candidate lies inside its range"));
        }
        fresh.push(Candidate { at: rate.tick_of(c.frame), score: score_of(c.score)? });
    }
    let walls = &mut draft.set.walls;
    walls.candidates.retain(|c| {
        let f = rate.frame_at(c.at);
        f < from || f >= to
    });
    for c in fresh {
        if !walls.stored.iter().any(|s| s.at == c.at) && !walls.candidates.iter().any(|x| x.at == c.at) {
            walls.candidates.push(c);
        }
    }
    walls.candidates.sort_by_key(|c| rate.frame_at(c.at));
    if walls.candidates.len() > MAX_CANDIDATES {
        return Err(too_many("candidates", walls.candidates.len(), MAX_CANDIDATES));
    }
    Ok(())
}

/// Adds `[from, to)` to the analysed ranges, merged and sorted.
fn add_analysed(set: &mut VerticalSet, rate: FrameRate, from: i64, to: i64) {
    let mut ranges: Vec<[i64; 2]> = set.walls.analysed.iter().map(|a| [rate.frame_at(a.from), rate.frame_at(a.to)]).collect();
    ranges.push([from, to]);
    set.walls.analysed = merge_ranges(ranges).into_iter().map(|[a, b]| FrameRange { from: rate.tick_of(a), to: rate.tick_of(b) }).collect();
}

/// `vertical.applyMeasurement` (spec section 4.2): inside `[from, to)` the stored `detected` walls and the candidates
/// become the measurement's; every other stored wall and every suppression stays; a cut on a suppressed frame or on
/// another stored wall is not applied; a cut inside a locked move stays a candidate (info `wall_blocked_by_locked`); a cut
/// inside another move follows W5; the range joins `analysed`.
pub(crate) fn apply_measurement(ctx: &Ctx, draft: &mut Draft, w: &ApplyMeasurement) -> Result<(), Refusal> {
    let m = &w.measurement;
    check_len("cuts", m.cuts.len(), MAX_CANDIDATES)?;
    check_len("candidates", m.candidates.len(), MAX_CANDIDATES)?;
    let (from, to) = (w.from_frame.unwrap_or(m.from_frame), w.to_frame.unwrap_or(m.to_frame));
    let end = ctx.r.end_frame;
    let inside_measurement = from >= m.from_frame && to <= m.to_frame;
    if from < 0 || from >= to || to > end || !inside_measurement {
        let detail = json!({"key": "fromFrame", "range": [from, to], "measured": [m.from_frame, m.to_frame]});
        return Err(Refusal::new("out_of_range", "the range lies inside the measurement's and the sequence", detail));
    }
    for c in m.cuts.iter().chain(&m.candidates) {
        wall_frame(ctx, c.frame)?;
        score_of(c.score)?;
    }
    let rate = ctx.r.rate;
    let in_range = |f: i64| f >= from && f < to;
    let walls = &mut draft.set.walls;
    walls.stored.retain(|s| !(s.origin == WallOrigin::Detected && in_range(rate.frame_at(s.at))));
    walls.candidates.retain(|c| !in_range(rate.frame_at(c.at)));
    let suppressed: Vec<i64> = walls.suppressed.iter().map(|t| rate.frame_at(*t)).collect();
    let mut blocked_cuts = Vec::new();
    for cut in m.cuts.iter().filter(|c| in_range(c.frame)) {
        let at = rate.tick_of(cut.frame);
        if suppressed.contains(&cut.frame) || draft.set.walls.stored.iter().any(|s| s.at == at) {
            continue;
        }
        let score = score_of(cut.score)?;
        if let Some((plan, position)) = locked_move_over(draft, ctx, cut.frame) {
            blocked_cuts.push(Candidate { at, score });
            let detail = json!({"plan": plan, "position": position, "frame": cut.frame});
            draft.issues.push(Issue { code: "wall_blocked_by_locked", level: Level::Info, plan: Some(plan), frames: Some([cut.frame, cut.frame + 1]), detail });
            continue;
        }
        draft.set.walls.stored.push(StoredWall { at, origin: WallOrigin::Detected, score: Some(score) });
    }
    let walls = &mut draft.set.walls;
    for c in m.candidates.iter().filter(|c| in_range(c.frame)) {
        let at = rate.tick_of(c.frame);
        if !walls.stored.iter().any(|s| s.at == at) {
            walls.candidates.push(Candidate { at, score: score_of(c.score)? });
        }
    }
    walls.candidates.extend(blocked_cuts);
    walls.candidates.sort_by_key(|c| rate.frame_at(c.at));
    walls.candidates.dedup_by_key(|c| c.at);
    if walls.candidates.len() > MAX_CANDIDATES {
        return Err(too_many("candidates", walls.candidates.len(), MAX_CANDIDATES));
    }
    sort_walls(&mut draft.set, rate);
    add_analysed(&mut draft.set, rate, from, to);
    split_moves(ctx, draft, w.on_conflict)?;
    let walls = &draft.set.walls;
    let detected = walls.stored.iter().filter(|s| s.origin == WallOrigin::Detected && in_range(rate.frame_at(s.at))).count();
    let possible = walls.candidates.iter().filter(|c| in_range(rate.frame_at(c.at))).count();
    draft.label = format!("Vertical: apply measured cuts {}\u{2013}{} (+{detected}, {possible} possible)", ctx.tc(from), ctx.tc(to));
    Ok(())
}

/// The plan and position of a locked move that contains `frame` (`s < frame <= f`), in any plan of the draft.
fn locked_move_over(draft: &Draft, ctx: &Ctx, frame: i64) -> Option<(u32, u32)> {
    for plan in &draft.set.plans {
        for k in 1..plan.positions.len() {
            let Some(span) = move_span(plan, &ctx.r, k) else { continue };
            let Some(p) = plan.positions.get(k) else { continue };
            if p.locked && span.start_frame < frame && frame <= span.end_frame {
                return Some((plan.id, p.id));
            }
        }
    }
    None
}
