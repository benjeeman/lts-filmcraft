//! Parts (spec section 4.6). T-M18, the model half: part boundaries are walls; `P07 00:00:12:03` parses; the status
//! passes through open, reviewed, approved, changed since approval and output changed since approval; an approval is
//! invalidated by a change and restored by undo (the snapshot before the change). And the part writes: content-idempotent
//! adds (`part_exists {id}` for the same name with other content), `moveEdges` moving shared edges together, and
//! `fromMarkers` copying segmentation markers once.

use serde_json::json;

use super::fixtures::*;
use crate::vertical::*;
use crate::{Label, Marker, MarkerId, MarkerKind};

const A: Window = Window { cx: 700.0, cy: 540.0, h: 1080.0 };
const B: Window = Window { cx: 1100.0, cy: 540.0, h: 1080.0 };

fn add(name: &str, from: i64, to: i64) -> PartAdd {
    PartAdd { name: name.into(), from_frame: from, to_frame: to, ..Default::default() }
}

fn parts(adds: Vec<PartAdd>) -> Write {
    Write::SetParts(SetParts { add: adds, ..Default::default() })
}

#[test]
fn t_m18_part_boundaries_are_walls() {
    let mut fx = Fx::new(2_000);
    let mut set = one_plan(vec![at25(1, 0, A, Arrival::Cut)]);
    set.parts = vec![part25(7, "P07", 100, 400)];
    fx.install(set);
    assert_eq!(fx.wall_frames(), vec![100, 400]);
    assert_eq!(wall_at(&fx, 400).expect("a part wall").sources, vec![WallSource::Part]);
    let e = fx.refuse(set_at(450, PositionFields { arrival: Some(ArrivalKind::Move), move_frames: Some(100), ..keys(1100.0, 540.0, 1080.0) }));
    assert_eq!(e.code, "move_crosses_wall");
    assert_eq!((e.detail["wall"].clone(), e.detail["sources"].clone()), (json!(400), json!(["part"])));
}

/// A set with parts P08 [100, 200), "Part seven" [1000, 1100) and P07 [4703, 4985) (spec section 2.9's part).
fn timecoded() -> VerticalSet {
    let mut set = one_plan(Vec::new());
    set.parts = vec![part25(8, "P08", 100, 200), part25(9, "Part seven", 1_000, 1_100), part25(7, "P07", 4_703, 4_985)];
    set
}

#[test]
fn t_m18_a_part_timecode_names_a_frame_of_the_part() {
    let set = timecoded();
    let parse = |text: &str| parse_part_timecode(&set, RATE, false, text);
    assert_eq!(parse("P07 00:00:12:03"), Ok(Some(4_703 + 12 * 25 + 3)));
    assert_eq!(parse("p07 00:00:12:03"), Ok(Some(5_006)), "the name is matched ignoring case");
    assert_eq!(parse("  P07   12:03 "), Ok(Some(5_006)), "a partial timecode");
    assert_eq!(parse("P07 1203"), Ok(Some(5_006)), "bare digits read right-aligned as HHMMSSFF");
    assert_eq!(parse("P07 +15"), Ok(Some(4_718)), "relative to the part's first frame");
    assert_eq!(parse("Part seven 00:00:01:00"), Ok(Some(1_025)), "a name with a space");
    assert_eq!(parse("P08 00:00:00:00"), Ok(Some(100)));
    assert_eq!(parse("00:00:12:03"), Ok(None), "no part prefix: a plain timecode for the caller");
    let e = parse("P99 00:00:01:00").expect_err("no part P99");
    assert_eq!((e.code, e.detail["part"].clone()), ("not_found", json!("P99")));
    assert_eq!(parse("P07 soon").expect_err("not a timecode").code, "bad_time");
    // P07 holds frames 4703-4984: a frame of it formats back to the timecode that names it; frame 5006 lies past its end
    assert_eq!(parse("P07 00:00:03:22"), Ok(Some(4_800)));
    assert_eq!(format_part_timecode(&set, RATE, false, 4_800).as_deref(), Some("P07 00:00:03:22"));
    assert_eq!(format_part_timecode(&set, RATE, false, 5_006), None, "past the part's end");
    assert_eq!(format_part_timecode(&set, RATE, false, 150).as_deref(), Some("P08 00:00:02:00"));
    assert_eq!(format_part_timecode(&set, RATE, false, 300), None, "outside every part");
    // in overlapping parts the first by `from` is shown
    let mut overlapping = timecoded();
    overlapping.parts.insert(2, part25(10, "P10", 4_000, 4_800));
    assert_eq!(format_part_timecode(&overlapping, RATE, false, 4_703).as_deref(), Some("P10 00:00:28:03"));
}

#[test]
fn t_m18_status_transitions_and_an_approval_bound_to_both_digests() {
    let mut fx = Fx::new(1_000);
    let mut set = one_plan(vec![at25(1, 100, A, Arrival::Cut), at25(2, 200, B, Arrival::Cut)]);
    set.parts = vec![part25(1, "P01", 100, 300)];
    fx.install(set);
    let status = |fx: &Fx| part_status(&fx.project, fx.seq(), 1, 1);
    assert_eq!(status(&fx), Some(PartStatus::Open), "unreviewed positions");
    for p in &mut fx.plan_mut(1).positions {
        p.reviewed = true;
    }
    assert_eq!(status(&fx), Some(PartStatus::Reviewed));
    fx.plan_mut(1).positions[1].flags = vec![Flag::PictureChanged];
    assert_eq!(status(&fx), Some(PartStatus::Open), "a flag keeps a part open");
    fx.plan_mut(1).positions[1].flags.clear();
    let approve = Write::ApprovePart(ApprovePart { plan: 1, part: 1, note: "first cut".into(), by: "Dana Editor".into(), when: 1_790_000_000 });
    let planned = fx.apply(approve.clone());
    assert_eq!(planned.label, "Vertical: approve part P01");
    assert_eq!(status(&fx), Some(PartStatus::Approved));
    let approval = fx.plan(1).part_approvals[0].clone();
    assert_eq!(approval.framing_digest, range_digest(&fx.project, fx.seq(), fx.plan(1), 100, 300));
    assert_eq!(approval.output_digest, output_digest(&fx.project, fx.seq(), fx.plan(1), &fx.set().parts[0]));
    assert_eq!((approval.part, approval.by.as_str(), approval.when, approval.note.as_str()), (1, "Dana Editor", 1_790_000_000, "first cut"));
    let approved = fx.set().clone();
    // a framing change
    fx.apply(set_at(200, PositionFields { cx: Some(1_000.0), ..Default::default() }));
    assert_eq!(status(&fx), Some(PartStatus::ChangedSinceApproval));
    let found = issues(&fx.project, fx.seq(), 1, 0, 1_000);
    let changed = with_code(&found, "part_changed_since_approval");
    assert_eq!((changed.len(), changed.first().map(|i| i.level)), (1, Some(Level::Warning)));
    assert_eq!(changed[0].detail["part"], json!(1));
    // undo restores the snapshot, and the approval holds again: digests are content-based
    fx.install(approved);
    assert_eq!(status(&fx), Some(PartStatus::Approved));
    // an output change only: the plan's caption layout
    let captions = VerticalCaptions { max_lines: 3, ..Default::default() };
    fx.apply(Write::Update(UpdatePlan { plan: 1, captions: Some(captions), ..Default::default() }));
    assert_eq!(status(&fx), Some(PartStatus::OutputChangedSinceApproval));
    let found = issues(&fx.project, fx.seq(), 1, 0, 1_000);
    assert_eq!(with_code(&found, "part_output_changed_since_approval").len(), 1);
    // approving again replaces the plan's approval of the part
    fx.apply(approve);
    assert_eq!(fx.plan(1).part_approvals.len(), 1);
    assert_eq!(status(&fx), Some(PartStatus::Approved));
    assert_eq!(part_status(&fx.project, fx.seq(), 1, 9), None, "no part 9");
}

#[test]
fn a_shot_that_is_not_own_keeps_its_part_open_and_names_it_in_not_reviewed() {
    let mut fx = Fx::new(1_000);
    let mut set = one_plan(vec![at25(1, 100, A, Arrival::Cut), at25(2, 200, B, Arrival::Cut)]);
    set.parts = vec![part25(1, "P01", 100, 300)];
    set.walls.stored = vec![stored(250, WallOrigin::Manual, None)];
    fx.install(set);
    for p in &mut fx.plan_mut(1).positions {
        p.reviewed = true;
    }
    assert_eq!(part_status(&fx.project, fx.seq(), 1, 1), Some(PartStatus::Open), "the shot from 250 is carried");
    assert_eq!(unreviewed_shots(&fx.project, fx.seq(), 1, 1), vec![3], "shots: [0, 100), [100, 250), [250, 300), [300, end)");
    let e = fx.refuse(Write::ApprovePart(ApprovePart { plan: 1, part: 1, by: "Dana Editor".into(), when: 1, ..Default::default() }));
    assert_eq!((e.code, e.detail["shots"].clone()), ("not_reviewed", json!([3])));
}

#[test]
fn adds_are_content_idempotent_and_the_same_name_with_other_content_is_part_exists() {
    let mut fx = Fx::new(1_000);
    fx.install(one_plan(Vec::new()));
    let planned = fx.apply(parts(vec![add("P01", 0, 100), add("P02", 100, 250)]));
    assert_eq!(planned.ids, vec![1, 2]);
    assert_eq!(planned.label, "Vertical: parts (+2)");
    let again = accepted(fx.write(parts(vec![add("P01", 0, 100)])));
    assert!(!again.changed, "an identical add changes nothing");
    assert_eq!(again.ids, vec![1], "and returns the existing id");
    let e = fx.refuse(parts(vec![add("P01", 0, 90)]));
    assert_eq!((e.code, e.detail["id"].clone()), ("part_exists", json!(1)));
    let e = fx.refuse(parts(vec![add("p01", 0, 100)]));
    assert_eq!((e.code, e.detail["id"].clone()), ("part_exists", json!(1)), "names are unique ignoring case");
    let meta = PartMeta { title: "Dana walks".into(), description: String::new() };
    let e = fx.refuse(parts(vec![PartAdd { meta: Some(meta), ..add("P02", 100, 250) }]));
    assert_eq!(e.code, "part_exists", "other metadata is other content");
    let audio = PartAudio { fade_in_frames: 2, fade_out_frames: 4 };
    let same = accepted(fx.write(parts(vec![PartAdd { audio: None, ..add("P02", 100, 250) }])));
    assert!(!same.changed);
    let e = fx.refuse(parts(vec![PartAdd { audio: Some(audio), ..add("P02", 100, 250) }]));
    assert_eq!(e.code, "part_exists", "a part's own fades, even equal to the set's, are other content");
    assert_eq!(fx.set().parts.len(), 2);
    assert_eq!(fx.set().next_part_id, 3);
}

#[test]
fn updates_and_removes_change_parts_by_id() {
    let mut fx = Fx::new(1_000);
    fx.install(one_plan(Vec::new()));
    fx.apply(parts(vec![add("P01", 0, 100), add("P02", 100, 250)]));
    let fades = PartAudio { fade_in_frames: 0, fade_out_frames: 10 };
    let update = PartUpdate { part: 2, name: Some("Act 2".into()), to_frame: Some(300), audio: Some(Some(fades)), ..Default::default() };
    fx.apply(Write::SetParts(SetParts { update: vec![update], remove: vec![1], ..Default::default() }));
    let got: Vec<(u32, String, i64, i64)> = fx.set().parts.iter().map(|p| (p.id, p.name.clone(), RATE.frame_at(p.from), RATE.frame_at(p.to))).collect();
    assert_eq!(got, vec![(2, "Act 2".to_string(), 100, 300)]);
    assert_eq!(fx.set().parts[0].audio, Some(PartAudio { fade_in_frames: 0, fade_out_frames: 10 }));
    let e = fx.refuse(Write::SetParts(SetParts { update: vec![PartUpdate { part: 1, name: Some("x".into()), ..Default::default() }], ..Default::default() }));
    assert_eq!((e.code, e.detail["part"].clone()), ("not_found", json!(1)), "part 1 is gone");
    let fades = PartUpdate { part: 2, audio: Some(Some(PartAudio { fade_in_frames: 251, fade_out_frames: 4 })), ..Default::default() };
    assert_eq!(fx.refuse(Write::SetParts(SetParts { update: vec![fades], ..Default::default() })).code, "bad_part", "fades are 0 to 250 frames");
}

#[test]
fn move_edges_moves_every_edge_on_a_frame_so_shared_edges_keep_tiling() {
    let mut fx = Fx::new(1_000);
    fx.install(one_plan(Vec::new()));
    fx.apply(parts(vec![add("P01", 0, 100), add("P02", 100, 200), add("P03", 300, 400)]));
    let edges = |frame: i64, to: i64| Write::SetParts(SetParts { move_edges: vec![EdgeMove { frame, to }], ..Default::default() });
    fx.apply(edges(100, 120));
    let ranges: Vec<(i64, i64)> = fx.set().parts.iter().map(|p| (RATE.frame_at(p.from), RATE.frame_at(p.to))).collect();
    assert_eq!(ranges, vec![(0, 120), (120, 200), (300, 400)]);
    assert!(!accepted(fx.write(edges(100, 120))).changed, "a retried edge move finds no edge on frame 100");
    let e = fx.refuse(edges(200, 110));
    assert_eq!((e.code, e.detail["part"].clone()), ("bad_part", json!(2)), "a part would be shorter than a frame");
    // a moved edge is a wall change: a move it would cut follows W5
    let mut fx = Fx::new(1_000);
    fx.install(one_plan(vec![at25(1, 0, A, Arrival::Cut), at25(2, 200, B, mv_frames(60))]));
    fx.apply(parts(vec![add("P01", 0, 100)]));
    assert_eq!(fx.refuse(edges(100, 170)).code, "would_split_move");
    let shorten = Write::SetParts(SetParts { move_edges: vec![EdgeMove { frame: 100, to: 170 }], on_conflict: OnConflict::Shorten, ..Default::default() });
    fx.apply(shorten);
    assert_eq!(fx.position(1, 2).arrival, Arrival::Move { move_frames: 30, curve: Curve::Linear, split_of: None });
}

fn mv_frames(frames: u32) -> Arrival {
    Arrival::Move { move_frames: frames, curve: Curve::Linear, split_of: None }
}

fn marker(id: u64, start: i64, frames: i64, name: &str, kind: MarkerKind, color: Label) -> Marker {
    Marker { id: MarkerId(id), start: tk(start), duration: tk(frames), name: name.into(), comment: String::new(), kind, color }
}

#[test]
fn from_markers_copies_segmentation_markers_once() {
    let mut fx = Fx::new(2_000);
    fx.install(one_plan(Vec::new()));
    fx.seq_mut().markers = vec![
        marker(1, 0, 250, "Cold open", MarkerKind::Segmentation, Label::Forest),
        marker(2, 250, 750, "", MarkerKind::Segmentation, Label::Forest),
        marker(3, 300, 10, "a note", MarkerKind::Comment, Label::Forest),
        marker(4, 1_000, 500, "Act 2", MarkerKind::Segmentation, Label::Rose),
    ];
    let copy = |color: Option<Label>| Write::SetParts(SetParts { from_markers: Some(PartMarkers { color }), ..Default::default() });
    let rose = accepted(fx.write(copy(Some(Label::Rose)))).set.expect("a set");
    let names: Vec<String> = rose.parts.iter().map(|p| p.name.clone()).collect();
    assert_eq!(names, vec!["Act 2".to_string()], "only the segmentation markers of that colour");
    fx.apply(copy(None));
    let got: Vec<(String, i64, i64)> = fx.set().parts.iter().map(|p| (p.name.clone(), RATE.frame_at(p.from), RATE.frame_at(p.to))).collect();
    assert_eq!(
        got,
        vec![("Cold open".to_string(), 0, 250), ("P02".to_string(), 250, 1_000), ("Act 2".to_string(), 1_000, 1_500)],
        "an unnamed marker is named P and its number among the copied markers"
    );
    assert!(!accepted(fx.write(copy(None))).changed, "copying again changes nothing");
}
