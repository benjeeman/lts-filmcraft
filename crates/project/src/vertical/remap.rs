//! Re-delivered masters: `vertical.remap` (spec section 4.12). The caller's map says where each run of old frames now
//! starts; [`plan_remap`] runs the edit hook's machinery (section 4.8, steps 3-12) with that map in place of the anchor map
//! of steps 1-2: exact splits and anchors at every junction, a `reanchored` wall at each junction with no wall, every
//! frame-based datum moved, removed content taking its data with it (reported), new content inheriting, resumed content
//! opening on its own position, the re-check of I5, I6 and I9, the flags, and validation. Positions remapped exactly keep
//! `reviewed`; the ones the remap adds are `reanchored` and unreviewed. The editor aligns nothing: the map is the caller's.
//!
//! Every entry is a rigid run `[from_frame, to_frame) -> new_from`; entries are sorted and apart on both sides. Old frames
//! in no entry were removed; new frames in no entry are new content. The editor cannot see the relinked master's cuts, so
//! every junction is a wall. A map that breaks this is refused `bad_map {entry, reason}` with reason `empty`,
//! `out_of_range` (a negative frame, an old frame past `VE_MAX_FRAMES`, or a new range past the sequence's end), then,
//! against the entry before it, `unsorted` (either side goes back) or `overlapping` (either side overlaps it). The old
//! side is bounded by `VE_MAX_FRAMES`, not by the sequence's end: the relinked sequence may already be shorter than the
//! master that was framed. The engine's required `expect` is [`remap_digest`].
//!
//! A remap whose result would still break an invariant is refused `invalid_plan {violations}` and changes nothing (data
//! that was damaged before is repaired first; data past the end of a shorter relinked master is what a remap fixes, so
//! the set before the remap is not validated against the sequence). For a person (the only caller in M2-M4) a locked
//! position on removed content is removed and reported (`position_removed {plan, position, frame, locked}`); an agent
//! caller's `locked` refusal is M5's (plan 63, D-12).

use std::collections::{BTreeMap, BTreeSet};

use filmcraft_time::{FrameRate, Tick};
use serde_json::json;

use super::derive::{Facts, Scene, plan_issues_near, raster, touches};
use super::digest::{data_digest, digest_of, plan_digest};
use super::follow::{Carry, Mapping, Piece, carry_set};
use super::plan::{PlanDelta, Planned, empty_set, frames_changed, too_many};
use super::types::{Caller, Issue, Position, Raster, Refusal, VE_MAX_FRAMES, VerticalPlan, VerticalSet};
use super::validate::{plan_only_violations, set_violations};
use crate::{Project, Sequence};

/// The most entries one map may hold (`too_many {key: "map"}`).
pub const MAX_MAP: usize = 10_000;

/// One entry of a remap: the old frames `[from_frame, to_frame)` now start at frame `new_from`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RemapEntry {
    pub from_frame: i64,
    pub to_frame: i64,
    pub new_from: i64,
}

/// Plans `vertical.remap` of `seq` with `map` for `caller`: the sequence's new vertical data and what changed (a sequence
/// write: `plan` is None and `plans` lists each plan's delta), or a refusal that changes nothing. A map that changes
/// nothing (the identity) is a no-op: `changed` false and no label. The label reads "Vertical: remap (3 ranges, 50
/// frames removed, 50 frames inserted)": the old frames and the new frames in no entry.
///
/// The engine plans it on its working copy of the sequence, whose id counters it raised to the session's high-water
/// marks (plan 63, C-11), as for every other write: the ids the remap hands out come from those counters.
pub fn plan_remap(project: &Project, seq: &Sequence, map: &[RemapEntry], caller: Caller) -> Result<Planned, Refusal> {
    let r = raster(seq);
    check_map(map, r.end_frame)?;
    let Some(set) = seq.vertical.as_deref() else { return Ok(Planned::default()) };
    let pieces = map.iter().map(|e| Piece::exact([e.from_frame, e.to_frame], e.new_from.saturating_sub(e.from_frame))).collect();
    let mapping = Mapping::new(pieces, i64::from(VE_MAX_FRAMES), r.end_frame);
    let carry = Carry { project, seq, old: set, r0: r, r1: r, map: &mapping, picture: None, writer: caller.writer };
    let carried = carry_set(&carry);
    if carried.set == *set {
        return Ok(Planned { set: Some(Box::new(set.clone())), ..Planned::default() });
    }
    let (plans, issues) = {
        let scene = Scene::new(project, seq, Some(&carried.set));
        let mut violations = set_violations(&scene, &carried.set);
        for plan in &carried.set.plans {
            violations.extend(plan_only_violations(&scene, plan));
        }
        if !violations.is_empty() {
            let detail = json!({"violations": violations});
            return Err(Refusal::new("invalid_plan", "the remap would leave data that breaks an invariant; repair it first", detail));
        }
        let mut plans = Vec::new();
        let mut issues: Vec<Issue> = carried.issues.clone();
        for after in &carried.set.plans {
            let Some(before) = set.plans.iter().find(|p| p.id == after.id) else { continue };
            let delta = delta_of(&r, before, after);
            if delta.added.is_empty() && delta.updated.is_empty() && delta.removed.is_empty() && delta.changed_frames.is_empty() {
                continue;
            }
            // the issues touching the changed frames and the positions added or updated, as every write reports them
            let mut ranges = delta.changed_frames.clone();
            let touched = after.positions.iter().filter(|p| delta.added.contains(&p.id) || delta.updated.contains(&p.id));
            ranges.extend(touched.map(|p| r.rate.frame_at(p.at)).map(|f| [f, f.saturating_add(1)]));
            let facts = Facts::new(&scene, after);
            for issue in plan_issues_near(&scene, &facts, Some(ranges.as_slice())).into_iter().filter(|i| touches(i, &ranges)) {
                if !issues.contains(&issue) {
                    issues.push(issue);
                }
            }
            plans.push(delta);
        }
        (plans, issues)
    };
    let label = label(map, data_end(set, r.rate), r.end_frame);
    let set = Some(Box::new(carried.set));
    Ok(Planned { set, changed: true, plan: None, plans, issues, changes: carried.changes, label, ..Planned::default() })
}

/// The guard of `vertical.remap` (its required `expect`): every plan's digest, in plan order, the set's data digest, and
/// the rest of what a remap moves: the walls, parts and regions (a plan's digest covers them too, but a set can hold them
/// before it has a plan) and the conform reference. Any change of the set's framing or data makes a retried remap
/// `stale_plan`; the id counters are no content. FNV-1a 64, printed `fnv1a64:` and 16 hex digits.
pub fn remap_digest(seq: &Sequence) -> String {
    let empty = empty_set();
    let set = seq.vertical.as_deref().unwrap_or(&empty);
    let plans: Vec<String> = set.plans.iter().map(|p| plan_digest(seq, p)).collect();
    let moved = (&set.walls, &set.parts, &set.regions, &set.reference);
    digest_of(&(plans, data_digest(set), moved))
}

/// The map checked (spec section 4.12): at most [`MAX_MAP`] entries (`too_many {key: "map"}`), then per entry in order
/// `bad_map {entry, reason}`: `empty`, `out_of_range`, and against the entry before it `unsorted` or `overlapping`.
fn check_map(map: &[RemapEntry], end: i64) -> Result<(), Refusal> {
    if map.len() > MAX_MAP {
        return Err(too_many("map", map.len(), MAX_MAP));
    }
    let mut previous: Option<RemapEntry> = None;
    for (index, e) in map.iter().enumerate() {
        let frames = e.to_frame.saturating_sub(e.from_frame);
        let outside = e.from_frame < 0 || e.new_from < 0 || e.to_frame > i64::from(VE_MAX_FRAMES) || e.new_from.saturating_add(frames) > end;
        let back = previous.is_some_and(|p| e.from_frame < p.from_frame || e.new_from < p.new_from);
        let over = previous.is_some_and(|p| e.from_frame < p.to_frame || e.new_from < p.new_from.saturating_add(p.to_frame - p.from_frame));
        let reason = if frames <= 0 {
            "empty"
        } else if outside {
            "out_of_range"
        } else if back {
            "unsorted"
        } else if over {
            "overlapping"
        } else {
            previous = Some(*e);
            continue;
        };
        let detail = json!({"entry": index, "reason": reason});
        return Err(Refusal::new("bad_map", format!("map entry {index} is {reason}"), detail));
    }
    Ok(())
}

/// What a remap did to one plan's positions: ids added, changed and removed, and the frames whose window or layout differ.
fn delta_of(r: &Raster, before: &VerticalPlan, after: &VerticalPlan) -> PlanDelta {
    let old: BTreeMap<u32, &Position> = before.positions.iter().map(|p| (p.id, p)).collect();
    let new: BTreeSet<u32> = after.positions.iter().map(|p| p.id).collect();
    let (mut added, mut updated) = (Vec::new(), Vec::new());
    for p in &after.positions {
        match old.get(&p.id) {
            None => added.push(p.id),
            Some(b) if *b != p => updated.push(p.id),
            Some(_) => {}
        }
    }
    let removed = before.positions.iter().map(|p| p.id).filter(|id| !new.contains(id)).collect();
    PlanDelta { plan: after.id, added, updated, removed, changed_frames: frames_changed(r, before, after) }
}

/// The undo label: "Vertical: remap (3 ranges, 50 frames removed, 50 frames inserted)", each count only when it is not
/// zero ("1 range", "1 frame"). Removed are the old frames in no entry: before and between the entries, and after the last
/// one up to `old_end`, the end of the old frames the set's data reaches ([`data_end`]: the editor does not know the old
/// master's length). Inserted are the new frames in no entry, up to `new_end`, the sequence's end.
fn label(map: &[RemapEntry], old_end: i64, new_end: i64) -> String {
    let (mut removed, mut inserted, mut old_at, mut new_at) = (0_i64, 0_i64, 0_i64, 0_i64);
    for e in map {
        removed = removed.saturating_add(e.from_frame.saturating_sub(old_at));
        inserted = inserted.saturating_add(e.new_from.saturating_sub(new_at));
        old_at = e.to_frame;
        new_at = e.new_from.saturating_add(e.to_frame.saturating_sub(e.from_frame));
    }
    removed = removed.saturating_add(old_end.saturating_sub(old_at).max(0));
    inserted = inserted.saturating_add(new_end.saturating_sub(new_at).max(0));
    let count = |n: i64, one: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {one}s") };
    let ranges = i64::try_from(map.len()).unwrap_or(i64::MAX);
    let removed = (removed > 0).then(|| format!("{} removed", count(removed, "frame")));
    let inserted = (inserted > 0).then(|| format!("{} inserted", count(inserted, "frame")));
    let parts: Vec<String> = [Some(count(ranges, "range")), removed, inserted].into_iter().flatten().collect();
    format!("Vertical: remap ({})", parts.join(", "))
}

/// One past the last old frame the set's data names: its walls, suppressions, candidates and analysed ranges, parts,
/// regions, annotations, box-track keys, active areas and reference frames, and each plan's positions, attention marks
/// and overwritten ranges.
fn data_end(set: &VerticalSet, rate: FrameRate) -> i64 {
    let at = |t: &Tick| rate.frame_at(*t);
    let after = |t: &Tick| rate.frame_at(*t).saturating_add(1);
    let w = &set.walls;
    let mut ends: Vec<i64> = Vec::new();
    ends.extend(w.stored.iter().map(|x| after(&x.at)).chain(w.suppressed.iter().map(after)).chain(w.candidates.iter().map(|x| after(&x.at))));
    ends.extend(w.analysed.iter().map(|x| at(&x.to)).chain(set.parts.iter().map(|x| at(&x.to))).chain(set.regions.iter().map(|x| at(&x.to))));
    ends.extend(set.annotations.iter().map(|x| at(&x.to)).chain(set.active_area.iter().map(|x| at(&x.to))));
    ends.extend(set.box_tracks.iter().flat_map(|t| &t.keys).map(|k| after(&k.at)));
    ends.extend(set.reference.iter().flat_map(|r| &r.frames).map(|f| f.saturating_add(1)));
    for plan in &set.plans {
        ends.extend(plan.positions.iter().map(|p| after(&p.at)).chain(plan.attention.iter().map(|m| after(&m.at))));
        ends.extend(plan.overwritten.iter().map(|o| at(&o.to)));
    }
    ends.into_iter().max().unwrap_or(0)
}
