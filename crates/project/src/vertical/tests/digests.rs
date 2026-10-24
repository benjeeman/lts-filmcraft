//! T-M35: what each digest covers (spec section 2.6). `digest` covers the plan and the set's walls, parts and regions and
//! never the id counters, annotations, box tracks or active areas; `dataDigest` covers exactly those three; `wallsDigest`
//! the stored wall set. Each is a pure function of the content, printed `fnv1a64:` and 16 lowercase hex digits.

use filmcraft_time::Tick;

use super::support::*;
use crate::Sequence;
use crate::vertical::*;

/// The plan digest of the first plan, the data digest and the walls digest of `seq`'s set.
fn digests(seq: &Sequence) -> [String; 3] {
    let set = seq.vertical.as_deref().expect("the sequence has a set");
    [plan_digest(seq, &set.plans[0]), data_digest(set), walls_digest(set)]
}

/// Which of the three digests change when `edit` changes the full set.
fn changed(edit: impl Fn(&mut VerticalSet)) -> [bool; 3] {
    let before = sequence_with(full_set(&hd()));
    let mut after = before.clone();
    if let Some(set) = after.vertical.as_deref_mut() {
        edit(set);
    }
    let (a, b) = (digests(&before), digests(&after));
    [a[0] != b[0], a[1] != b[1], a[2] != b[2]]
}

#[test]
fn t_m35_digests_print_fnv1a64_and_16_hex_digits() {
    for d in digests(&sequence_with(full_set(&hd()))) {
        assert!(digest_ok(&d), "{d}");
    }
    assert_eq!(digest::digest_text(0x0123_4567_89ab_cdef), "fnv1a64:0123456789abcdef");
    assert_eq!(digest::digest_text(7), "fnv1a64:0000000000000007");
    // the FNV-1a 64 test vectors
    assert_eq!(digest::fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(digest::fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    assert_eq!(digest::fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
}

#[test]
fn t_m35_the_plan_digest_covers_the_plan_walls_parts_and_regions() {
    let plan_only = [true, false, false];
    assert_eq!(changed(|s| s.plans[0].positions[0].window.cx += 0.001), plan_only);
    assert_eq!(changed(|s| s.plans[0].positions[1].reviewed = false), plan_only);
    assert_eq!(changed(|s| s.plans[0].name.push('x')), plan_only);
    assert_eq!(changed(|s| s.plans[0].captions.max_lines = 4), plan_only);
    assert_eq!(changed(|s| s.plans[0].starting_window = None), plan_only);
    assert_eq!(changed(|s| s.plans[0].part_approvals[0].note.push('!')), plan_only);
    assert_eq!(changed(|s| s.plans[0].attention[0].note.push('!')), plan_only);
    assert_eq!(changed(|s| s.plans[0].overwritten[0].label.push('!')), plan_only);
    assert_eq!(changed(|s| s.parts[0].name.push('x')), plan_only);
    assert_eq!(changed(|s| s.parts[0].audio = None), plan_only);
    assert_eq!(changed(|s| s.regions[0].rect[0] += 1.0), plan_only);
    assert_eq!(changed(|s| s.regions.truncate(2)), plan_only);
    // the walls are in both the plan digest and the walls digest
    assert_eq!(changed(|s| s.walls.use_edit_points = false), [true, false, true]);
    assert_eq!(changed(|s| s.walls.suppressed.push(Tick(1))), [true, false, true]);
    assert_eq!(changed(|s| s.walls.candidates.clear()), [true, false, true]);
}

#[test]
fn t_m35_the_data_digest_covers_annotations_box_tracks_and_active_areas() {
    let data_only = [false, true, false];
    assert_eq!(changed(|s| s.annotations[0].label.push('x')), data_only);
    assert_eq!(changed(|s| s.annotations.truncate(1)), data_only);
    assert_eq!(changed(|s| s.box_tracks[0].keys[0].rect[0] += 1.0), data_only);
    assert_eq!(changed(|s| s.box_tracks[0].keys[1].gone = false), data_only);
    assert_eq!(changed(|s| s.active_area[0].rect[1] += 1.0), data_only);
}

#[test]
fn t_m35_counters_and_records_change_no_digest() {
    let none = [false, false, false];
    assert_eq!(changed(|s| s.next_plan_id += 1), none);
    assert_eq!(changed(|s| s.next_part_id += 1), none);
    assert_eq!(changed(|s| s.next_region_id += 1), none);
    assert_eq!(changed(|s| s.next_annotation_id += 1), none);
    assert_eq!(changed(|s| s.plans[0].next_position_id += 1), none);
    assert_eq!(changed(|s| s.plans[0].next_record_id += 1), none);
    assert_eq!(changed(|s| s.agent_overwrite = None), none);
    assert_eq!(changed(|s| s.deliveries.clear()), none);
    assert_eq!(changed(|s| s.reference = None), none);
    assert_eq!(changed(|s| s.part_audio.fade_in_frames += 1), none);
    assert_eq!(changed(|s| s.delivery.series.push('x')), none);
    // another plan is not part of the first plan's digest
    assert_eq!(changed(|s| s.plans[1].name.push('x')), none);
}

#[test]
fn t_m35_digests_are_pure_functions_of_the_content() {
    let (a, b) = (sequence_with(full_set(&hd())), sequence_with(full_set(&hd())));
    let (da, db) = (digests(&a), digests(&b));
    assert!(da.iter().all(|d| digest_ok(d.as_str())), "{da:?}");
    assert_eq!(da, db, "equal content, equal digests");
    // the three digests differ from each other, and from those of an empty set
    assert!(da[0] != da[1] && da[1] != da[2] && da[0] != da[2], "{da:?}");
    let mut empty = full_set(&hd());
    empty.annotations.clear();
    empty.box_tracks.clear();
    empty.active_area.clear();
    assert_ne!(data_digest(&empty), da[1]);
    // a change and its undo give the digest back
    let mut changed_back = full_set(&hd());
    changed_back.plans[0].positions[0].window.cx += 1.0;
    changed_back.plans[0].positions[0].window.cx -= 1.0;
    assert_eq!(digests(&sequence_with(changed_back)), da);
}

#[test]
fn t_m35_the_digests_hash_their_scope_as_it_prints() {
    fn hash(text: &str) -> String {
        digest::digest_text(digest::fnv1a64(text.as_bytes()))
    }
    let seq = sequence_with(full_set(&hd()));
    let set = seq.vertical.as_deref().expect("the sequence has a set");
    let plan = &set.plans[0];
    // the plan as the project file prints it without its two counters (they print last), then the set's walls, parts
    // and regions, so a reader can recompute the digest; a plan field the digest left out would fail here
    let printed = serde_json::to_string(plan).expect("prints");
    let counters = format!(",\"nextPositionId\":{},\"nextRecordId\":{}}}", plan.next_position_id, plan.next_record_id);
    let content = printed.strip_suffix(counters.as_str()).unwrap_or_default();
    assert!(!content.is_empty() && !content.contains("nextPositionId"), "{printed}");
    let walls = serde_json::to_string(&set.walls).expect("prints");
    let parts = serde_json::to_string(&set.parts).expect("prints");
    let regions = serde_json::to_string(&set.regions).expect("prints");
    let scope = format!("{{\"plan\":{content}}},\"walls\":{walls},\"parts\":{parts},\"regions\":{regions}}}");
    assert_eq!(plan_digest(&seq, plan), hash(&scope));
    // the data digest: the set's annotations, box tracks and active areas as they print; the walls digest: the walls
    let annotations = serde_json::to_string(&set.annotations).expect("prints");
    let tracks = serde_json::to_string(&set.box_tracks).expect("prints");
    let areas = serde_json::to_string(&set.active_area).expect("prints");
    assert_eq!(data_digest(set), hash(&format!("{{\"annotations\":{annotations},\"boxTracks\":{tracks},\"activeArea\":{areas}}}")));
    assert_eq!(walls_digest(set), hash(&walls));
}
