//! T-M30, `vertical.remap` at model level (spec section 4.12), the person's clauses: a map that removes 50 frames inside a
//! move and inserts 25 elsewhere keeps every surviving frame's window within two split bounds; every junction is a wall;
//! removed content's positions are reported, a locked one too (it goes, for a person); every frame-based datum moves,
//! suppressions included; `bad_map` names the entry and the reason; a remapped part's approval reads
//! `changedSinceApproval`; an identity map changes nothing. The agent's `locked` refusal is M5's. Synthetic data only.

use filmcraft_time::Tick;
use serde_json::json;

use super::fixtures::{Fx, at25, one_plan, part25, tk};
use super::hook_fixtures::*;
use super::support::{OUT, digest_ok, mv, mv_split, same_bits, win};
use crate::Project;
use crate::vertical::*;

fn entry(from_frame: i64, to_frame: i64, new_from: i64) -> RemapEntry {
    RemapEntry { from_frame, to_frame, new_from }
}

/// The map of T-M30 on a 600-frame master: frames 200-249 go (inside the move into #3), and 25 new frames come at 450
/// (the old frame 500 resumes at 475). New frames 575-599 lie past the last entry: new content too.
fn tm30_map() -> Vec<RemapEntry> {
    vec![entry(0, 200, 0), entry(250, 500, 200), entry(500, 600, 475)]
}

/// Where the map sends each old frame.
fn tm30_oracle() -> Vec<Option<i64>> {
    (0..600)
        .map(|f| {
            if f < 200 {
                Some(f)
            } else if f < 250 {
                None
            } else if f < 500 {
                Some(f - 50)
            } else {
                Some(f - 25)
            }
        })
        .collect()
}

/// Plans the remap as a person (a refusal fails the test) and applies its result to a copy of the project.
fn remapped(fx: &Fx, map: &[RemapEntry]) -> (Planned, Project) {
    let planned = match plan_remap(&fx.project, fx.seq(), map, PERSON) {
        Ok(planned) => planned,
        Err(e) => panic!("refused {}: {} {}", e.code, e.message, e.detail),
    };
    let set = planned.set.clone();
    (planned, edited(&fx.project, |p| p.sequence_mut(fx.seq).expect("the sequence").vertical = set))
}

/// The ids of plan 1's positions that the remap removed (its delta), sorted.
fn removed_ids(planned: &Planned) -> Vec<u32> {
    let mut ids: Vec<u32> = planned.plans.iter().filter(|d| d.plan == 1).flat_map(|d| d.removed.iter().copied()).collect();
    ids.sort_unstable();
    ids
}

fn frames(ticks: impl Iterator<Item = Tick>) -> Vec<i64> {
    ticks.map(fr).collect()
}

#[test]
fn t_m30_a_map_removing_50_frames_inside_a_move_and_inserting_25_keeps_every_window() {
    let mut fx = Fx::with_clips(&[(0, 600, 0)]);
    let held = win(900.0, 540.0, 1080.0);
    fx.install(one_plan(vec![
        at25(1, 0, win(600.0, 540.0, 1080.0), Arrival::Cut),
        at25(2, 100, win(800.0, 540.0, 1080.0), Arrival::Cut),
        at25(3, 300, win(1300.0, 540.0, 900.0), mv(150, Curve::EaseInOut)),
        at25(4, 450, held, Arrival::Cut),
        at25(5, 550, win(1000.0, 540.0, 1080.0), mv(40, Curve::Linear)),
    ]));
    let (planned, after) = remapped(&fx, &tm30_map());
    assert!(planned.changed);
    assert_eq!(planned.plan, None, "a remap is a sequence write");
    // 25 new frames at 450 and the 25 after the last entry (575-599) are in no entry: 50 inserted
    assert_eq!(planned.label, "Vertical: remap (3 ranges, 50 frames removed, 50 frames inserted)");
    let (r, old, new) = (fx.r(), plan_in(&fx.project, fx.seq, 1), plan_in(&after, fx.seq, 1));
    assert_eq!(frames_of(new), vec![0, 100, 199, 200, 250, 400, 475, 525]);
    assert_eq!(ids_of(new), vec![1, 2, 6, 7, 3, 4, 8, 5], "the added positions take the next ids, in frame order");
    let exact = |f: i64| clamp_window(window_at(old, &r, tk(f)).window, &r, OUT);
    let (left, right, opener) = (find(new, 6).expect("#6"), find(new, 7).expect("#7"), find(new, 8).expect("#8"));
    assert!(same_bits(&left.window, &exact(199)));
    assert_eq!(left.arrival, mv_split(49, Curve::EaseInOut, 0, 150));
    assert!(same_bits(&right.window, &exact(250)));
    assert_eq!(right.arrival, Arrival::Cut, "no move crosses the junction");
    assert_eq!(find(new, 3).expect("#3").arrival, mv_split(50, Curve::EaseInOut, 100, 150));
    assert!(same_bits(&opener.window, &held), "the resumed content opens on an exact copy of #4's window");
    assert_eq!(opener.arrival, Arrival::Cut);
    for p in [left, right, opener] {
        assert_eq!((p.origin, p.reviewed), (Origin::Reanchored, false), "#{}", p.id);
    }
    assert_eq!(find(new, 5).expect("#5").arrival, mv(40, Curve::Linear), "a move inside one entry keeps its length");
    assert_eq!(stored_walls(&after, fx.seq, WallOrigin::Reanchored), vec![200, 450, 475, 575], "every junction is a wall");
    assert_follows(&fx.project, &after, fx.seq, 1, &tm30_oracle(), "the remap");
    assert!(validate(&after, seq_of(&after, fx.seq)).is_empty());
    let delta = planned.plans.iter().find(|d| d.plan == 1).expect("plan 1 changed");
    let mut added = delta.added.clone();
    added.sort_unstable();
    assert_eq!(added, vec![6, 7, 8]);
    assert!(removed_ids(&planned).is_empty());
}

#[test]
fn t_m30_the_label_counts_the_frames_after_the_last_entry_too() {
    let mut fx = Fx::with_clips(&[(0, 600, 0)]);
    fx.install(one_plan(vec![
        at25(1, 0, win(600.0, 540.0, 1080.0), Arrival::Cut),
        at25(2, 100, win(800.0, 540.0, 1080.0), Arrival::Cut),
        at25(3, 550, win(1000.0, 540.0, 1080.0), Arrival::Cut),
    ]));
    // the old frames from 500 on are in no entry, as far as the data reaches (#3 at 550); so are the new frames 500-599
    let (planned, after) = remapped(&fx, &[entry(0, 500, 0)]);
    assert_eq!(planned.label, "Vertical: remap (1 range, 51 frames removed, 100 frames inserted)");
    assert_eq!(frames_of(plan_in(&after, fx.seq, 1)), vec![0, 100]);
    assert_eq!(removed_ids(&planned), vec![3]);
}

#[test]
fn t_m30_every_frame_based_datum_moves_suppressions_included() {
    let mut fx = Fx::with_clips(&[(0, 600, 0)]);
    let mut set = one_plan(vec![at25(1, 0, win(600.0, 540.0, 1080.0), Arrival::Cut), at25(2, 100, win(800.0, 540.0, 1080.0), Arrival::Cut)]);
    set.walls.stored = vec![StoredWall { at: tk(350), origin: WallOrigin::Manual, score: None }];
    set.walls.suppressed = vec![tk(120), tk(220)];
    set.walls.candidates = vec![Candidate { at: tk(480), score: 0.5 }];
    set.walls.analysed = vec![FrameRange { from: tk(0), to: tk(600) }];
    set.parts = vec![part25(1, "P01", 0, 150), part25(2, "P02", 400, 600)];
    set.next_part_id = 3;
    set.regions = vec![ProtectedRegion {
        id: 1,
        label: "logo".into(),
        from: tk(100),
        to: tk(300),
        rect: [100.0, 100.0, 200.0, 100.0],
        mode: RegionMode::KeepIn,
        origin: Origin::Manual,
        modified_by: Writer::Human,
    }];
    set.next_region_id = 2;
    set.annotations = vec![annotation(1, "speech", 180, 260)];
    set.next_annotation_id = 2;
    set.box_tracks = vec![dana(vec![key(190), key(240), key(260)])];
    set.active_area = vec![ActiveArea { from: tk(100), to: tk(300), rect: [0.0, 132.0, 1920.0, 816.0], origin: Origin::Manual, modified_by: Writer::Human }];
    set.plans[0].attention = vec![mark(210), mark(260)];
    let grids = vec!["AAAA".to_string(), "BBBB".to_string(), "CCCC".to_string(), "DDDD".to_string()];
    set.reference = Some(ConformReference { when: 1_790_000_300, media: Vec::new(), frames: vec![100, 199, 230, 260], grids, wall_pairs: Vec::new() });
    fx.install(set);
    let (planned, after) = remapped(&fx, &tm30_map());
    let set = set_in(&after, fx.seq);
    let stored: Vec<(i64, WallOrigin)> = set.walls.stored.iter().map(|w| (fr(w.at), w.origin)).collect();
    let reanchored = WallOrigin::Reanchored;
    assert_eq!(stored, vec![(200, reanchored), (300, WallOrigin::Manual), (450, reanchored), (475, reanchored)], "575 is P02's end: a part wall");
    assert_eq!(frames(set.walls.suppressed.iter().copied()), vec![120], "the suppression on removed content goes");
    let dropped: Vec<&Change> = planned.changes.iter().filter(|c| c.code == "suppression_dropped").collect();
    assert_eq!(dropped.len(), 1);
    assert_eq!(dropped[0].detail["frame"], json!(220), "and it is reported");
    assert_eq!(set.walls.candidates, vec![Candidate { at: tk(430), score: 0.5 }]);
    let analysed: Vec<(i64, i64)> = set.walls.analysed.iter().map(|a| (fr(a.from), fr(a.to))).collect();
    assert_eq!(analysed, vec![(0, 450), (475, 575)], "the inserted frames were never analysed");
    let parts: Vec<(u32, i64, i64)> = set.parts.iter().map(|p| (p.id, fr(p.from), fr(p.to))).collect();
    assert_eq!(parts, vec![(1, 0, 150), (2, 350, 575)]);
    assert_eq!((fr(set.regions[0].from), fr(set.regions[0].to)), (100, 250));
    let notes: Vec<(u32, i64, i64)> = set.annotations.iter().map(|a| (a.id, fr(a.from), fr(a.to))).collect();
    assert_eq!(notes, vec![(1, 180, 200), (2, 200, 210)], "the speech splits at the junction");
    assert_eq!(set.next_annotation_id, 3);
    assert_eq!(frames(set.box_tracks[0].keys.iter().map(|k| k.at)), vec![190, 210]);
    let areas: Vec<(i64, i64)> = set.active_area.iter().map(|a| (fr(a.from), fr(a.to))).collect();
    assert_eq!(areas, vec![(100, 200), (200, 250)]);
    assert_eq!(frames(set.plans[0].attention.iter().map(|m| m.at)), vec![210]);
    let reference = set.reference.as_ref().expect("the reference stays");
    assert_eq!(reference.frames, vec![100, 199, 210]);
    assert_eq!(reference.grids, vec!["AAAA".to_string(), "BBBB".to_string(), "DDDD".to_string()]);
    assert!(validate(&after, seq_of(&after, fx.seq)).is_empty());
}

#[test]
fn t_m30_bad_map_names_the_entry_and_the_reason() {
    let mut fx = Fx::with_clips(&[(0, 600, 0)]);
    fx.install(one_plan(vec![at25(1, 0, win(600.0, 540.0, 1080.0), Arrival::Cut)]));
    let cases: Vec<(Vec<RemapEntry>, usize, &str)> = vec![
        (vec![entry(0, 100, 0), entry(50, 150, 100)], 1, "overlapping"),
        (vec![entry(100, 200, 100), entry(0, 50, 0)], 1, "unsorted"),
        (vec![entry(0, 100, 0), entry(100, 200, 50)], 1, "overlapping"),
        (vec![entry(0, 100, 200), entry(100, 200, 0)], 1, "unsorted"),
        (vec![entry(0, 100, 550)], 0, "out_of_range"),
        (vec![entry(-10, 100, 0)], 0, "out_of_range"),
        (vec![entry(0, 100, -5)], 0, "out_of_range"),
        (vec![entry(100, 100, 0)], 0, "empty"),
        (vec![entry(0, 100, 0), entry(200, 150, 300)], 1, "empty"),
    ];
    for (map, index, reason) in cases {
        let e = match plan_remap(&fx.project, fx.seq(), &map, PERSON) {
            Ok(planned) => panic!("{map:?} was accepted: {}", planned.label),
            Err(e) => e,
        };
        assert_eq!(e.code, "bad_map", "{map:?}");
        assert_eq!((e.detail["entry"].clone(), e.detail["reason"].clone()), (json!(index), json!(reason)), "{map:?}");
    }
    let long: Vec<RemapEntry> = (0..10_001).map(|i| entry(i, i + 1, i)).collect();
    let e = plan_remap(&fx.project, fx.seq(), &long, PERSON).expect_err("a map holds at most 10 000 entries");
    assert_eq!((e.code, e.detail["key"].clone()), ("too_many", json!("map")));
}

#[test]
fn t_m30_a_remapped_part_reads_changed_since_approval_and_an_untouched_one_stays_approved() {
    let mut fx = Fx::with_clips(&[(0, 600, 0)]);
    let reviewed = |p: Position| Position { reviewed: true, ..p };
    let mut set = one_plan(vec![
        reviewed(at25(1, 0, win(600.0, 540.0, 1080.0), Arrival::Cut)),
        reviewed(at25(2, 100, win(800.0, 540.0, 1080.0), Arrival::Cut)),
        reviewed(at25(3, 400, win(1100.0, 540.0, 1080.0), Arrival::Cut)),
    ]);
    set.parts = vec![part25(1, "P01", 0, 150), part25(2, "P02", 400, 600)];
    set.next_part_id = 3;
    fx.install(set);
    for id in [1, 2] {
        approve(&mut fx, 1, id);
        assert_eq!(part_status(&fx.project, fx.seq(), 1, id), Some(PartStatus::Approved));
    }
    let (_, after) = remapped(&fx, &tm30_map());
    let seq = seq_of(&after, fx.seq);
    assert_eq!(part_status(&after, seq, 1, 2), Some(PartStatus::ChangedSinceApproval), "approvals bind to digests that hold times");
    assert_eq!(part_status(&after, seq, 1, 1), Some(PartStatus::Approved), "P01's frames did not move");
}

#[test]
fn t_m30_a_person_remapping_removes_a_locked_position_on_removed_content_and_reports_it() {
    let mut fx = Fx::with_clips(&[(0, 600, 0)]);
    fx.install(one_plan(vec![
        at25(1, 0, win(600.0, 540.0, 1080.0), Arrival::Cut),
        Position { locked: true, ..at25(6, 220, win(1000.0, 540.0, 1080.0), Arrival::Cut) },
        at25(2, 300, win(800.0, 540.0, 1080.0), Arrival::Cut),
    ]));
    let (planned, after) = remapped(&fx, &tm30_map());
    assert!(find(plan_in(&after, fx.seq, 1), 6).is_none(), "its picture is gone");
    assert_eq!(removed_ids(&planned), vec![6]);
    let removed: Vec<&Change> = planned.changes.iter().filter(|c| c.code == "position_removed").collect();
    assert_eq!(removed.len(), 1);
    let detail = ["plan", "position", "frame", "locked"].map(|k| removed[0].detail[k].clone());
    assert_eq!(detail, [json!(1), json!(6), json!(220), json!(true)], "the old frame, and that it was locked");
}

#[test]
fn t_m30_an_identity_map_changes_nothing_and_a_shift_moves_everything() {
    let mut fx = Fx::with_clips(&[(0, 600, 0)]);
    fx.install(one_plan(vec![at25(1, 0, win(600.0, 540.0, 1080.0), Arrival::Cut), at25(2, 100, win(800.0, 540.0, 1080.0), mv(40, Curve::Linear))]));
    let same = plan_remap(&fx.project, fx.seq(), &[entry(0, 600, 0)], PERSON).expect("the identity map");
    assert!(!same.changed);
    assert_eq!(same.label, "");
    assert_eq!(same.set.as_deref(), fx.seq().vertical.as_deref());
    let (shift, after) = remapped(&fx, &[entry(0, 599, 1)]);
    assert!(shift.changed);
    assert_eq!(frames_of(plan_in(&after, fx.seq, 1)), vec![1, 101], "every position moves a frame");
}

#[test]
fn t_m30_the_remap_digest_covers_every_plan_and_the_data() {
    let mut fx = Fx::with_clips(&[(0, 600, 0)]);
    fx.install(one_plan(vec![at25(1, 0, win(600.0, 540.0, 1080.0), Arrival::Cut)]));
    let seq = fx.seq().clone();
    let d0 = remap_digest(&seq);
    assert!(digest_ok(&d0), "{d0}");
    assert_eq!(remap_digest(&seq.clone()), d0, "a pure function of the content");
    let with = |edit: &dyn Fn(&mut VerticalSet)| {
        let mut s = seq.clone();
        edit(s.vertical.as_deref_mut().expect("the set"));
        remap_digest(&s)
    };
    assert_ne!(with(&|set: &mut VerticalSet| set.plans[0].positions[0].window.cx = 610.0), d0, "a window");
    assert_ne!(with(&|set: &mut VerticalSet| set.walls.suppressed.push(tk(300))), d0, "a suppression");
    assert_ne!(with(&|set: &mut VerticalSet| set.annotations.push(annotation(1, "speech", 10, 20))), d0, "an annotation");
    assert_eq!(with(&|set: &mut VerticalSet| set.plans[0].next_position_id = 99), d0, "a counter is no content");
}
