//! Parts (spec section 4.6): named ranges a person or an agent defines, each one short. Their boundaries are walls (source
//! `part`); the editor never chooses, moves or proposes an edge. This module holds the part writes of
//! `vertical.setParts` (`moveEdges`, `fromMarkers`, content-idempotent adds, `part_exists`), `vertical.approvePart`, the
//! part timecode (`P07 00:00:12:03`) and the part status per plan.
//!
//! Part names are unique ignoring ASCII case, because the part timecode matches a name case-insensitively.

use filmcraft_time::{FrameRate, Tick, format_timecode_frames, parse_timecode};
use serde::Serialize;
use serde_json::json;

use super::derive::{Facts, Scene, ShotStatus, error_issues, shots_of};
use super::digest::range_digest_in;
use super::output_digest::output_digest_in;
use super::plan::{ApprovePart, Ctx, Draft, MAX_LIST, SetParts, alloc, chars_within, check_len, not_found_id, not_found_plan, too_many};
use super::types::{Part, PartApproval, PartMeta, Refusal, VerticalPlan, VerticalSet};
use super::walls::{counted, shown_frame, split_moves};
use crate::{Label, Marker, MarkerKind, Project, Sequence};

/// The most parts a set holds (I14).
pub const MAX_PARTS: usize = 500;

/// The longest part name, title and description (I13, I14).
const MAX_NAME: usize = 64;
const MAX_TITLE: usize = 200;
const MAX_DESCRIPTION: usize = 5_000;

/// The longest edge fade, in frames (I14).
pub const MAX_FADE: u32 = 250;

/// The longest approval note (I13).
const MAX_NOTE: usize = 1_000;

/// A part's status in one plan (spec section 4.6), derived, never stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PartStatus {
    /// A shot in the part has unreviewed positions or flags, or is not own.
    Open,
    /// Every position reviewed, no flags, no error issue, every shot own; no approval.
    Reviewed,
    /// An approval whose framing digest and output digest both equal the part's current ones.
    Approved,
    /// The approval's framing digest differs from the part's range digest (`part_changed_since_approval`).
    ChangedSinceApproval,
    /// The framing digest is equal but the output digest differs (`part_output_changed_since_approval`).
    OutputChangedSinceApproval,
}

/// The frame a timecode with a part prefix names: `"P07 00:00:12:03"` is part P07's first frame plus 12 s 3 frames (the
/// name matched ignoring ASCII case, the longest name that is followed by white space; then a timecode or partial
/// timecode as `parse_timecode` reads it, relative forms counting from the part's start). `Ok(None)` when the text has no
/// part prefix (the caller reads it as a plain timecode); `not_found {part}` for a prefix that names no part;
/// `bad_time {timecode}` for a timecode that does not parse.
pub fn parse_part_timecode(set: &VerticalSet, rate: FrameRate, drop_frame: bool, input: &str) -> Result<Option<i64>, Refusal> {
    let text = input.trim();
    let named = |part: &&Part| {
        let n = part.name.len();
        let head = text.get(..n).is_some_and(|h| h.eq_ignore_ascii_case(&part.name));
        head && text.get(n..).is_some_and(|rest| rest.starts_with(char::is_whitespace))
    };
    let Some(part) = set.parts.iter().filter(named).max_by_key(|p| p.name.len()) else {
        let Some(split) = text.rfind(char::is_whitespace) else { return Ok(None) };
        let name = text.get(..split).unwrap_or_default().trim();
        return Err(Refusal::new("not_found", format!("no part is named {name:?}"), json!({"part": name})));
    };
    let rest = text.get(part.name.len()..).unwrap_or_default().trim();
    let frames = parse_timecode(rest, rate, drop_frame, 0).map_err(|e| {
        let detail = json!({"timecode": input, "reason": e.to_string()});
        Refusal::new("bad_time", format!("{rest:?} is not a timecode"), detail)
    })?;
    Ok(Some(rate.frame_at(part.from).saturating_add(frames)))
}

/// The part timecode of `frame` in the first part (by `from`) that contains it: `"P07 00:00:12:03"`, frame minus the part's
/// first frame, formatted from 00:00:00:00 at `rate` (drop-frame as the sequence). None outside every part.
pub fn format_part_timecode(set: &VerticalSet, rate: FrameRate, drop_frame: bool, frame: i64) -> Option<String> {
    let holds = |p: &&Part| rate.frame_at(p.from) <= frame && frame < rate.frame_at(p.to);
    let part = set.parts.iter().filter(holds).min_by_key(|p| (rate.frame_at(p.from), p.id))?;
    let offset = frame.saturating_sub(rate.frame_at(part.from));
    Some(format!("{} {}", part.name, format_timecode_frames(offset, rate, drop_frame)))
}

/// The status of part `part` in plan `plan` of `seq`. None when either does not exist.
pub fn part_status(project: &Project, seq: &Sequence, plan: u32, part: u32) -> Option<PartStatus> {
    let scene = Scene::of(project, seq);
    let p = scene.plan(plan)?;
    let part = scene.set?.parts.iter().find(|x| x.id == part)?;
    part_status_in(&scene, p, part)
}

/// The status of a part in a plan of a scene.
pub(crate) fn part_status_in(scene: &Scene, plan: &VerticalPlan, part: &Part) -> Option<PartStatus> {
    if let Some(approval) = plan.part_approvals.iter().find(|a| a.part == part.id) {
        return Some(approval_status(scene, plan, part, approval));
    }
    Some(if unreviewed_in(scene, plan, part).is_empty() { PartStatus::Reviewed } else { PartStatus::Open })
}

/// How an approval stands against the part's current digests: the framing digest is compared first.
pub(crate) fn approval_status(scene: &Scene, plan: &VerticalPlan, part: &Part, approval: &PartApproval) -> PartStatus {
    let rate = scene.r.rate;
    if approval.framing_digest != range_digest_in(scene, plan, rate.frame_at(part.from), rate.frame_at(part.to)) {
        return PartStatus::ChangedSinceApproval;
    }
    if approval.output_digest != output_digest_in(scene, plan, part) {
        return PartStatus::OutputChangedSinceApproval;
    }
    PartStatus::Approved
}

/// The shots of part `part` (1-based shot numbers of the plan) that keep it from being reviewed: an unreviewed or flagged
/// position, a shot that is not own, or an error issue in its frames. Empty when the part is reviewed; the detail of
/// `not_reviewed {shots}`.
pub fn unreviewed_shots(project: &Project, seq: &Sequence, plan: u32, part: u32) -> Vec<u32> {
    let scene = Scene::of(project, seq);
    let Some(p) = scene.plan(plan) else { return Vec::new() };
    let Some(part) = scene.set.and_then(|s| s.parts.iter().find(|x| x.id == part)) else { return Vec::new() };
    unreviewed_in(&scene, p, part)
}

/// The shots of a part that keep it from being reviewed, in a scene.
pub(crate) fn unreviewed_in(scene: &Scene, plan: &VerticalPlan, part: &Part) -> Vec<u32> {
    let rate = scene.r.rate;
    let (from, to) = (rate.frame_at(part.from), rate.frame_at(part.to));
    let facts = Facts::new(scene, plan);
    let errors = error_issues(scene, &facts);
    let mut out = Vec::new();
    for shot in shots_of(scene, &facts) {
        let (lo, hi) = (shot.from.max(from), shot.to.min(to));
        if lo >= hi {
            continue;
        }
        let unreviewed = plan.positions.iter().filter(|p| (lo..hi).contains(&rate.frame_at(p.at))).any(|p| !p.reviewed || !p.flags.is_empty());
        let error = errors.iter().any(|e| e.frames.is_none_or(|[a, b]| a < hi && lo < b));
        if shot.status != ShotStatus::Own || unreviewed || error {
            out.push(shot.number);
        }
    }
    out
}

/// A part's content checked: `too_many` for long text, `bad` (`bad_part`) for an empty name, a range outside `[0, end]`
/// or not from before to, or a fade over 250 frames.
fn check_part(ctx: &Ctx, part: &Part, bad: &dyn Fn(&str) -> Refusal) -> Result<(), Refusal> {
    chars_within("name", &part.name, MAX_NAME)?;
    chars_within("title", &part.meta.title, MAX_TITLE)?;
    chars_within("description", &part.meta.description, MAX_DESCRIPTION)?;
    if part.name.is_empty() {
        return Err(bad("a part has a name"));
    }
    let rate = ctx.r.rate;
    let (from, to) = (rate.frame_at(part.from), rate.frame_at(part.to));
    if from < 0 || from >= to || to > ctx.r.end_frame {
        return Err(bad("a part's frames lie inside [0, end], from before to"));
    }
    if part.audio.is_some_and(|a| a.fade_in_frames > MAX_FADE || a.fade_out_frames > MAX_FADE) {
        return Err(bad("a fade is 0 to 250 frames"));
    }
    Ok(())
}

/// `part_exists {id}` when another part than `except` has `name`, ignoring ASCII case.
fn name_free(set: &VerticalSet, name: &str, except: Option<u32>) -> Result<(), Refusal> {
    match set.parts.iter().find(|p| Some(p.id) != except && p.name.eq_ignore_ascii_case(name)) {
        Some(other) => Err(Refusal::new("part_exists", format!("a part is named {:?} already", other.name), json!({"id": other.id}))),
        None => Ok(()),
    }
}

/// `vertical.setParts`, applied in the order of [`SetParts`], then W5 for every new boundary inside a move.
pub(crate) fn set_parts(ctx: &Ctx, draft: &mut Draft, w: &SetParts) -> Result<(), Refusal> {
    for (key, len) in [("add", w.add.len()), ("update", w.update.len()), ("remove", w.remove.len()), ("moveEdges", w.move_edges.len())] {
        check_len(key, len, MAX_LIST)?;
    }
    let rate = ctx.r.rate;
    let before = draft.set.parts.len();
    draft.set.parts.retain(|p| !w.remove.contains(&p.id));
    let removed = before - draft.set.parts.len();
    for u in &w.update {
        let Some(current) = draft.set.parts.iter().find(|p| p.id == u.part).cloned() else { return Err(not_found_id("part", u.part)) };
        let bad = |reason: &str| Refusal::new("bad_part", format!("part {}: {reason}", u.part), json!({"part": u.part, "reason": reason}));
        let from = u.from_frame.unwrap_or(rate.frame_at(current.from));
        let to = u.to_frame.unwrap_or(rate.frame_at(current.to));
        if from < 0 || from >= to || to > ctx.r.end_frame {
            return Err(bad("a part's frames lie inside [0, end], from before to"));
        }
        let name = u.name.clone().unwrap_or_else(|| current.name.clone());
        let audio = u.audio.unwrap_or(current.audio);
        let meta = u.meta.clone().unwrap_or_else(|| current.meta.clone());
        let next = Part { id: current.id, name, from: rate.tick_of(from), to: rate.tick_of(to), audio, meta };
        check_part(ctx, &next, &bad)?;
        name_free(&draft.set, &next.name, Some(next.id))?;
        if let Some(slot) = draft.set.parts.iter_mut().find(|p| p.id == u.part) {
            *slot = next;
        }
    }
    let mut added = 0;
    for (index, a) in w.add.iter().enumerate() {
        let bad = |reason: &str| Refusal::new("bad_part", format!("part {index}: {reason}"), json!({"index": index, "reason": reason}));
        chars_within("name", &a.name, MAX_NAME)?;
        if a.from_frame < 0 || a.from_frame >= a.to_frame || a.to_frame > ctx.r.end_frame {
            return Err(bad("a part's frames lie inside [0, end], from before to"));
        }
        let meta = a.meta.clone().unwrap_or_default();
        let wanted = Part { id: 0, name: a.name.clone(), from: rate.tick_of(a.from_frame), to: rate.tick_of(a.to_frame), audio: a.audio, meta };
        check_part(ctx, &wanted, &bad)?;
        let same = |p: &&Part| p.name == wanted.name && p.from == wanted.from && p.to == wanted.to && p.audio == wanted.audio && p.meta == wanted.meta;
        if let Some(existing) = draft.set.parts.iter().find(same) {
            draft.ids.push(existing.id);
            continue;
        }
        name_free(&draft.set, &wanted.name, None)?;
        let top = draft.set.parts.iter().map(|p| p.id).max().unwrap_or(0);
        let id = alloc(&mut draft.set.next_part_id, top)?;
        draft.set.parts.push(Part { id, ..wanted });
        draft.ids.push(id);
        added += 1;
    }
    if let Some(filter) = &w.from_markers {
        added += from_markers(ctx, draft, filter.color)?;
    }
    for m in &w.move_edges {
        let end = ctx.r.end_frame;
        if !(0..=end).contains(&m.frame) || !(0..=end).contains(&m.to) {
            let detail = json!({"key": "moveEdges", "frame": m.frame, "to": m.to, "end": end});
            return Err(Refusal::new("out_of_range", "an edge lies on a frame inside [0, end]", detail));
        }
        let (at, to) = (rate.tick_of(m.frame), rate.tick_of(m.to));
        for p in &mut draft.set.parts {
            let (moves_from, moves_to) = (p.from == at, p.to == at);
            if !moves_from && !moves_to {
                continue;
            }
            if moves_from {
                p.from = to;
            }
            if moves_to {
                p.to = to;
            }
            if p.from >= p.to {
                let detail = json!({"part": p.id, "reason": "a part would be shorter than a frame"});
                return Err(Refusal::new("bad_part", format!("part {} would be shorter than a frame", p.id), detail));
            }
        }
    }
    if draft.set.parts.len() > MAX_PARTS {
        return Err(too_many("parts", draft.set.parts.len(), MAX_PARTS));
    }
    draft.set.parts.sort_by_key(|p| (rate.frame_at(p.from), p.id));
    split_moves(ctx, draft, w.on_conflict)?;
    draft.label = counted("parts", added, removed);
    Ok(())
}

/// `fromMarkers {kind: "segmentation", color}`: copies, once, the segmentation markers (of that colour) as parts named
/// after them, or "P" and their 2-digit 1-based index among the copied markers when unnamed. A marker whose name a part
/// has already, or whose frames make no part, is skipped. Returns how many parts it added.
fn from_markers(ctx: &Ctx, draft: &mut Draft, color: Option<Label>) -> Result<usize, Refusal> {
    let rate = ctx.r.rate;
    let wanted = |m: &&Marker| m.kind == MarkerKind::Segmentation && color.is_none_or(|c| c == m.color);
    let mut markers: Vec<&Marker> = ctx.seq.markers.iter().filter(wanted).collect();
    markers.sort_by_key(|m| (m.start, m.id));
    let mut added = 0;
    for (n, marker) in markers.into_iter().enumerate() {
        let name = if marker.name.is_empty() { format!("P{:02}", n + 1) } else { marker.name.clone() };
        let from = shown_frame(rate, marker.start).max(0);
        let to = shown_frame(rate, Tick(marker.start.0.saturating_add(marker.duration.0))).min(ctx.r.end_frame);
        let taken = draft.set.parts.iter().any(|p| p.name.eq_ignore_ascii_case(&name));
        if taken || from >= to || name.chars().count() > MAX_NAME {
            continue;
        }
        let top = draft.set.parts.iter().map(|p| p.id).max().unwrap_or(0);
        let id = alloc(&mut draft.set.next_part_id, top)?;
        let (from, to) = (rate.tick_of(from), rate.tick_of(to));
        draft.set.parts.push(Part { id, name, from, to, audio: None, meta: PartMeta::default() });
        added += 1;
    }
    Ok(added)
}

/// `vertical.approvePart`: refused `not_reviewed {shots}` unless the part is reviewed; stores the approval with both
/// digests, replacing the plan's approval of the part. The same digests, name and note again is a retry: a no-op.
pub(crate) fn approve_part(ctx: &Ctx, draft: &mut Draft, w: &ApprovePart) -> Result<(), Refusal> {
    chars_within("note", &w.note, MAX_NOTE)?;
    let scene = Scene::new(ctx.project, ctx.seq, Some(&draft.set));
    let plan = scene.plan(w.plan).ok_or_else(|| not_found_plan(w.plan))?;
    let part = draft.set.parts.iter().find(|p| p.id == w.part).ok_or_else(|| not_found_id("part", w.part))?;
    let shots = unreviewed_in(&scene, plan, part);
    if !shots.is_empty() {
        let detail = json!({"part": part.id, "shots": shots});
        return Err(Refusal::new("not_reviewed", format!("part {} has shots that are not reviewed", part.name), detail));
    }
    let rate = ctx.r.rate;
    let framing_digest = range_digest_in(&scene, plan, rate.frame_at(part.from), rate.frame_at(part.to));
    let output_digest = output_digest_in(&scene, plan, part);
    let approval = PartApproval { part: part.id, framing_digest, output_digest, by: w.by.clone(), when: w.when, note: w.note.clone() };
    let name = part.name.clone();
    let Some(target) = draft.set.plans.iter_mut().find(|p| p.id == w.plan) else { return Err(not_found_plan(w.plan)) };
    // the same approval again, but for its time, is a retry
    let retry = |a: &PartApproval| PartApproval { when: approval.when, ..a.clone() } == approval;
    match target.part_approvals.iter_mut().find(|a| a.part == w.part) {
        Some(slot) if retry(slot) => {}
        Some(slot) => *slot = approval,
        None => target.part_approvals.push(approval),
    }
    draft.label = format!("Vertical: approve part {name}");
    Ok(())
}
