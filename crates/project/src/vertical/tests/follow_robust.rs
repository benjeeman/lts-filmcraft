//! T-M11 and T-M33, the edit hook on data that is not ordinary (lane L1c round 3): a plan that breaks an invariant still
//! follows its picture; counters with no id left are reported and the positions around a missing split are flagged; a
//! very long retimed clip is matched by intervals; ids come from `before`'s counters, which the engine raises to the
//! session's high-water marks; a part with no frame left takes its approvals with it; active areas that would overlap are
//! cut back and reported. Synthetic data only.

use serde_json::json;

use super::fixtures::{Fx, at25, one_plan, part25, tk};
use super::hook_fixtures::*;
use super::support::{mv, win};
use crate::vertical::*;

/// A full-height window at `cx`.
fn full(cx: f64) -> Window {
    win(cx, 540.0, 1080.0)
}

/// The worked example of spec section 4.8: `P_i` at 100 and `P_j` at 150, joined by a 50-frame ease, over one clip.
fn worked() -> Fx {
    let mut fx = Fx::with_clips(&[(0, 400, 0)]);
    fx.install(one_plan(vec![at25(1, 100, full(812.4), Arrival::Cut), at25(2, 150, full(960.0), mv(50, Curve::EaseInOut))]));
    fx
}

#[test]
fn t_m11_a_plan_that_breaks_an_invariant_still_follows_its_picture() {
    let mut fx = Fx::with_clips(&[(0, 400, 0)]);
    fx.install(one_plan(vec![at25(1, 50, full(1500.0), Arrival::Cut), at25(2, 200, full(960.0), Arrival::Cut)]));
    let before = fx.project.clone();
    // 1440 wide: #1's window, made on 1920, lies outside the frame now (I3)
    let mut narrow = edited(&before, |p| p.sequence_mut(fx.seq).expect("the sequence").settings.width = 1440);
    follow(&before, &mut narrow);
    assert!(!validate(&narrow, seq_of(&narrow, fx.seq)).is_empty(), "the plan breaks I3");
    // a ripple delete: the plan follows its picture all the same
    let mut after = edited(&narrow, |p| ripple_delete(p, fx.seq, 100, 120));
    let report = follow(&narrow, &mut after);
    let new = plan_in(&after, fx.seq, 1);
    assert_eq!(fr(find(new, 2).expect("#2").at), 180, "#2 moves with its picture");
    assert_eq!(new.framed_on, Some([1920, 1080]), "the windows still belong to 1920x1080");
    assert_eq!(plan_report(&report, fx.seq, 1).moved, vec![2]);
    let codes: Vec<&str> = report.sequences.iter().flat_map(|s| &s.issues).map(|i| i.code).collect();
    assert!(codes.contains(&"invalid_plan"), "what the result still breaks is reported: {codes:?}");
}

#[test]
fn t_m11_with_no_id_left_the_hook_reports_it_and_flags_the_positions_around_the_missing_split() {
    let mut fx = worked();
    fx.plan_mut(1).next_position_id = u32::MAX;
    fx.set_mut().annotations = vec![annotation(1, "speech", 110, 140)];
    fx.set_mut().next_annotation_id = u32::MAX;
    let before = fx.project.clone();
    let mut after = edited(&before, |p| ripple_delete(p, fx.seq, 120, 130));
    let report = follow(&before, &mut after);
    let new = plan_in(&after, fx.seq, 1);
    assert_eq!((ids_of(new), frames_of(new)), (vec![1, 2], vec![100, 140]), "no split could be added");
    for id in [1, 2] {
        assert!(find(new, id).expect("kept").flags.contains(&Flag::PictureChanged), "#{id} now governs frames a split would have held");
    }
    let kinds: Vec<serde_json::Value> = report_changes(&report, fx.seq, "ids_exhausted").iter().map(|c| c.detail["kind"].clone()).collect();
    assert_eq!(kinds, vec![json!("annotation"), json!("position")]);
    assert_eq!(set_in(&after, fx.seq).annotations.len(), 1, "the speech keeps the piece that holds its id");
}

#[test]
fn t_m11_a_long_retimed_clip_is_matched_by_intervals() {
    // 100 000 000 frames, 46 days at 25p: matched frame by frame, the retime took a table of 10^8 rows
    const LONG: i64 = 100_000_000;
    let mut fx = Fx::with_clips(&[(0, LONG, 0)]);
    fx.install(one_plan(vec![
        at25(1, 0, full(600.0), Arrival::Cut),
        at25(2, 1_000, full(800.0), Arrival::Cut),
        at25(3, 50_000_000, full(1200.0), mv(100, Curve::Linear)),
        at25(4, 99_999_000, full(1000.0), Arrival::Cut),
    ]));
    let a = clip_ids(&fx.project, fx.seq)[0];
    let before = fx.project.clone();
    let mut after = edited(&before, |p| retime(p, fx.seq, a, 2.0, LONG / 2));
    follow(&before, &mut after);
    let new = plan_in(&after, fx.seq, 1);
    assert_eq!(frames_of(new), vec![0, 500, 25_000_000, 49_999_500]);
    assert_eq!(find(new, 3).expect("#3").arrival, mv(50, Curve::Linear), "the move keeps its 4 s");
    assert!(new.positions.iter().all(|p| !p.flags.contains(&Flag::TimeApproximate)), "every position lands on its media frame");
    assert!(validate(&after, seq_of(&after, fx.seq)).is_empty());
}

#[test]
fn t_m11_the_hook_hands_out_ids_from_befores_counters_which_the_engine_raises_to_its_marks() {
    // ids 3 to 9 were handed out and undone: the session's mark is 10, and the engine raised the counter of `before` to it
    let mut fx = worked();
    fx.plan_mut(1).next_position_id = 10;
    let before = fx.project.clone();
    let mut after = edited(&before, |p| ripple_delete(p, fx.seq, 120, 130));
    follow(&before, &mut after);
    let new = plan_in(&after, fx.seq, 1);
    assert_eq!(ids_of(new), vec![1, 10, 11, 2], "no id below the mark is handed out again");
    assert_eq!(new.next_position_id, 12);
    // an edit that adds no position leaves the counter as it was
    let clip = clip_ids(&before, fx.seq)[0];
    let mut trimmed = edited(&before, |p| trim_tail(p, fx.seq, clip, -10));
    follow(&before, &mut trimmed);
    assert_eq!(plan_in(&trimmed, fx.seq, 1).next_position_id, 10);
}

#[test]
fn t_m33_a_part_with_no_frame_left_takes_its_approvals_with_it() {
    let mut fx = Fx::with_clips(&[(0, 400, 0)]);
    let reviewed = |p: Position| Position { reviewed: true, ..p };
    let mut set = one_plan(vec![reviewed(at25(1, 0, full(600.0), Arrival::Cut)), reviewed(at25(2, 160, full(900.0), Arrival::Cut))]);
    set.parts = vec![part25(1, "P01", 0, 150), part25(2, "P02", 150, 200)];
    set.next_part_id = 3;
    fx.install(set);
    approve(&mut fx, 1, 1);
    approve(&mut fx, 1, 2);
    let before = fx.project.clone();
    let mut after = edited(&before, |p| ripple_delete(p, fx.seq, 150, 200));
    let report = follow(&before, &mut after);
    let set = set_in(&after, fx.seq);
    assert_eq!(set.parts.iter().map(|p| p.id).collect::<Vec<u32>>(), vec![1]);
    assert_eq!(set.plans[0].part_approvals.iter().map(|a| a.part).collect::<Vec<u32>>(), vec![1], "P02's approval goes with it");
    let gone = report_changes(&report, fx.seq, "approval_removed");
    assert_eq!(gone.len(), 1);
    assert_eq!((gone[0].detail["plan"].clone(), gone[0].detail["part"].clone()), (json!(1), json!(2)));
    assert!(validate(&after, seq_of(&after, fx.seq)).is_empty());
}

/// A person's active area over frames `[from, to)`, inside letterbox bars.
fn area(from: i64, to: i64) -> ActiveArea {
    ActiveArea { from: tk(from), to: tk(to), rect: [0.0, 132.0, 1920.0, 816.0], origin: Origin::Manual, modified_by: Writer::Human }
}

#[test]
fn t_m33_active_areas_that_would_overlap_are_cut_back_and_reported() {
    let mut fx = Fx::with_clips(&[(0, 200, 0)]);
    let mut set = one_plan(vec![at25(1, 0, full(600.0), Arrival::Cut)]);
    set.active_area = vec![area(0, 51), area(51, 100)];
    fx.install(set);
    let a = clip_ids(&fx.project, fx.seq)[0];
    let before = fx.project.clone();
    // at 200 % old frames 50 and 51 both land on 25: the first area ends at 26 now, and the second starts there
    let mut after = edited(&before, |p| retime(p, fx.seq, a, 2.0, 100));
    let report = follow(&before, &mut after);
    let areas: Vec<(i64, i64)> = set_in(&after, fx.seq).active_area.iter().map(|a| (fr(a.from), fr(a.to))).collect();
    assert_eq!(areas, vec![(0, 26), (26, 50)]);
    let trimmed = report_changes(&report, fx.seq, "area_trimmed");
    assert_eq!(trimmed.len(), 1);
    assert_eq!((trimmed[0].detail["old"].clone(), trimmed[0].detail["from"].clone()), (json!([51, 100]), json!(26)));
    assert!(validate(&after, seq_of(&after, fx.seq)).is_empty());
}
