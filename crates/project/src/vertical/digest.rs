//! Content digests (spec section 2.6): FNV-1a 64 over canonical bytes (`serde_json::to_vec` of the value, in struct field
//! order, id counters omitted), printed `fnv1a64:` and 16 lowercase hex digits, the policy's own format. Equal content gives
//! an equal digest, so a digest taken before an undo holds again after it.

use serde::Serialize;

use super::types::{
    ActiveArea, Annotation, AttentionMark, BoxTrack, Filter, FitBackground, OutputSize, OverwrittenRange, Part, PartApproval, Position, ProtectedRegion,
    VerticalCaptions, VerticalPlan, VerticalSet, WallSet, Window,
};
use crate::Sequence;

/// FNV-1a 64: its offset basis and its prime, as the policy's `targeting_digest` uses them.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a 64 of `bytes`.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    fold(FNV_OFFSET, bytes)
}

/// `fnv1a64:` and the 16 lowercase hex digits of `hash`.
pub fn digest_text(hash: u64) -> String {
    format!("fnv1a64:{hash:016x}")
}

/// The plan's `digest` (the `expect` of every write naming the plan): the plan (positions, settings, approvals, attention
/// marks, overwritten ranges) and the set's walls, parts and regions. Not the id counters, annotations, box tracks or
/// active areas.
pub fn plan_digest(seq: &Sequence, plan: &VerticalPlan) -> String {
    let set = seq.vertical.as_deref();
    let content = PlanContent {
        id: plan.id,
        name: &plan.name,
        output: plan.output,
        filter: plan.filter,
        starting_window: plan.starting_window,
        captions: &plan.captions,
        fit_background: plan.fit_background,
        positions: &plan.positions,
        part_approvals: &plan.part_approvals,
        attention: &plan.attention,
        overwritten: &plan.overwritten,
    };
    let parts = set.map(|s| s.parts.as_slice()).unwrap_or_default();
    let regions = set.map(|s| s.regions.as_slice()).unwrap_or_default();
    digest_of(&PlanScope { plan: content, walls: set.map(|s| &s.walls), parts, regions })
}

/// The set's `dataDigest`: its annotations, box tracks and active areas.
pub fn data_digest(set: &VerticalSet) -> String {
    digest_of(&DataScope { annotations: &set.annotations, box_tracks: &set.box_tracks, active_area: &set.active_area })
}

/// The set's `wallsDigest`: the stored wall set.
pub fn walls_digest(set: &VerticalSet) -> String {
    digest_of(&set.walls)
}

/// What a plan's digest covers: the plan without its id counters, and the walls, parts and regions of its sequence's set
/// (null and empty for a sequence without one).
#[derive(Serialize)]
struct PlanScope<'a> {
    plan: PlanContent<'a>,
    walls: Option<&'a WallSet>,
    #[serde(skip_serializing_if = "is_empty")]
    parts: &'a [Part],
    #[serde(skip_serializing_if = "is_empty")]
    regions: &'a [ProtectedRegion],
}

/// A plan as it prints in the project file, without `nextPositionId` and `nextRecordId`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanContent<'a> {
    id: u32,
    name: &'a str,
    output: OutputSize,
    filter: Filter,
    #[serde(skip_serializing_if = "Option::is_none")]
    starting_window: Option<Window>,
    captions: &'a VerticalCaptions,
    fit_background: FitBackground,
    positions: &'a [Position],
    #[serde(skip_serializing_if = "is_empty")]
    part_approvals: &'a [PartApproval],
    #[serde(skip_serializing_if = "is_empty")]
    attention: &'a [AttentionMark],
    #[serde(skip_serializing_if = "is_empty")]
    overwritten: &'a [OverwrittenRange],
}

/// What the data digest covers: the set's annotations, box tracks and active areas, as they print in the project file.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DataScope<'a> {
    #[serde(skip_serializing_if = "is_empty")]
    annotations: &'a [Annotation],
    #[serde(skip_serializing_if = "is_empty")]
    box_tracks: &'a [BoxTrack],
    #[serde(skip_serializing_if = "is_empty")]
    active_area: &'a [ActiveArea],
}

/// The digest of `value`'s canonical bytes. They stream into the hash as they print, so even a set with 50 000 box-track
/// keys allocates no buffer.
fn digest_of<T: Serialize>(value: &T) -> String {
    let mut hasher = Fnv1a(FNV_OFFSET);
    // Printing plain data (string keys, numbers or null) into a writer that never fails cannot fail; were it to, the hash
    // would cover the bytes printed before the failure, still a pure function of the value.
    let _ = serde_json::to_writer(&mut hasher, value);
    digest_text(hasher.0)
}

/// Whether a borrowed list is empty: the `skip_serializing_if` of the views' lists, as `Vec::is_empty` is the stored
/// types'.
fn is_empty<T>(items: &&[T]) -> bool {
    items.is_empty()
}

/// `hash` folded over `bytes`, one byte at a time: xor, then multiply by the prime.
fn fold(hash: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(hash, |hash, &byte| (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME))
}

/// The running FNV-1a 64 state, written into by `serde_json`.
struct Fnv1a(u64);

impl std::io::Write for Fnv1a {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = fold(self.0, bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
