//! T-M33 and T-M32 (mapping half): every frame-based datum follows the 16:9 edit (spec section 4.8, step 4). Stored walls,
//! suppressions, candidates, box-track keys, attention marks and reference frames are points: they move with their
//! picture and go with removed content. Annotations, active areas and overwritten ranges lose the frames that are no longer
//! shown and split at a junction (the first piece keeps the id). Analysed ranges keep exactly the frames that were
//! analysed (inserted content is not). Parts and regions take the span of their surviving frames. Approvals are never
//! changed: a part that moved reads `changedSinceApproval`. Synthetic data only.

use filmcraft_time::Tick;

use super::fixtures::{Fx, at25, one_plan, part25, tk};
use super::hook_fixtures::*;
use super::support::win;
use crate::vertical::*;

/// The frames of a list of ticks.
fn frames(ticks: impl Iterator<Item = Tick>) -> Vec<i64> {
    ticks.map(fr).collect()
}

/// A set over a 400-frame clip holding one plan (a cut at 0) and a datum of every kind.
fn data_set() -> VerticalSet {
    let mut set = one_plan(vec![at25(1, 0, win(600.0, 540.0, 1080.0), Arrival::Cut)]);
    set.walls.stored = vec![StoredWall { at: tk(300), origin: WallOrigin::Manual, score: None }];
    set.walls.suppressed = vec![tk(180)];
    set.walls.candidates = vec![Candidate { at: tk(350), score: 0.25 }];
    set.walls.analysed = vec![FrameRange { from: tk(0), to: tk(400) }];
    set.parts = vec![part25(1, "P01", 0, 150), part25(2, "P02", 150, 400)];
    set.next_part_id = 3;
    set.regions = vec![ProtectedRegion {
        id: 1,
        label: "logo".into(),
        from: tk(100),
        to: tk(200),
        rect: [100.0, 100.0, 200.0, 100.0],
        mode: RegionMode::KeepIn,
        origin: Origin::Manual,
        modified_by: Writer::Human,
    }];
    set.next_region_id = 2;
    set.annotations = vec![annotation(1, "speech", 100, 200), annotation(2, "music", 250, 300)];
    set.next_annotation_id = 3;
    set.box_tracks = vec![dana(vec![key(100), key(150), key(260)])];
    set.active_area = vec![ActiveArea { from: tk(50), to: tk(160), rect: [0.0, 132.0, 1920.0, 816.0], origin: Origin::Manual, modified_by: Writer::Human }];
    let plan = &mut set.plans[0];
    plan.attention = vec![mark(150), mark(270)];
    let reviewed = at25(9, 150, win(900.0, 540.0, 1080.0), Arrival::Cut);
    let label = "Vertical: replace 1 position (agent)".to_string();
    plan.overwritten = vec![OverwrittenRange { id: 1, from: tk(140), to: tk(180), positions: vec![reviewed], label, run_id: None, when: 1_790_000_200 }];
    plan.next_record_id = 2;
    let grids = vec!["AAAA".to_string(), "BBBB".to_string(), "CCCC".to_string()];
    set.reference = Some(ConformReference { when: 1_790_000_300, media: Vec::new(), frames: vec![100, 150, 260], grids, wall_pairs: Vec::new() });
    set
}

/// The ranges of a set (parts, regions, annotations, active areas, analysed ranges) as `(id, kind, from, to)` rows, to
/// compare them at once.
fn spans(set: &VerticalSet) -> Vec<(u32, &str, i64, i64)> {
    let mut rows: Vec<(u32, &str, i64, i64)> = set.parts.iter().map(|p| (p.id, "part", fr(p.from), fr(p.to))).collect();
    rows.extend(set.regions.iter().map(|g| (g.id, "region", fr(g.from), fr(g.to))));
    rows.extend(set.annotations.iter().map(|a| (a.id, a.kind.as_str(), fr(a.from), fr(a.to))));
    rows.extend(set.active_area.iter().map(|a| (0, "area", fr(a.from), fr(a.to))));
    rows.extend(set.walls.analysed.iter().map(|a| (0, "analysed", fr(a.from), fr(a.to))));
    rows
}

#[test]
fn t_m33_every_datum_follows_a_ripple_delete_and_removed_content_drops_it() {
    let mut fx = Fx::with_clips(&[(0, 400, 0)]);
    fx.install(data_set());
    let before = fx.project.clone();
    let mut after = edited(&before, |p| ripple_delete(p, fx.seq, 140, 160));
    follow(&before, &mut after);
    let set = set_in(&after, fx.seq);
    assert_eq!(set.walls.stored, vec![StoredWall { at: tk(280), origin: WallOrigin::Manual, score: None }]);
    assert_eq!(frames(set.walls.suppressed.iter().copied()), vec![160], "a suppression follows its picture");
    assert_eq!(set.walls.candidates, vec![Candidate { at: tk(330), score: 0.25 }]);
    assert_eq!(
        spans(set),
        vec![
            (1, "part", 0, 140),
            (2, "part", 140, 380),
            (1, "region", 100, 180),
            (1, "speech", 100, 140),
            (3, "speech", 140, 180),
            (2, "music", 230, 280),
            (0, "area", 50, 140),
            (0, "analysed", 0, 380),
        ],
        "the speech splits at the junction; analysed frames stay analysed"
    );
    assert_eq!(set.next_annotation_id, 4);
    assert_eq!(frames(set.box_tracks[0].keys.iter().map(|k| k.at)), vec![100, 240], "the key at 150 went");
    let plan = &set.plans[0];
    assert_eq!(frames(plan.attention.iter().map(|m| m.at)), vec![250], "the mark at 150 went");
    let range = &plan.overwritten[0];
    assert_eq!((range.id, fr(range.from), fr(range.to), range.positions.len()), (1, 140, 160, 0), "the range lost its removed frames");
    let reference = set.reference.as_ref().expect("the reference stays");
    assert_eq!(reference.frames, vec![100, 240], "a reference frame on removed content leaves the reference");
    assert_eq!(reference.grids, vec!["AAAA".to_string(), "CCCC".to_string()]);
    assert!(validate(&after, seq_of(&after, fx.seq)).is_empty());
}

#[test]
fn t_m33_every_datum_follows_a_ripple_insert_and_inserted_content_is_not_analysed() {
    let mut fx = Fx::with_clips(&[(0, 400, 0)]);
    let other = add_matte(&mut fx.project, "insert");
    fx.install(data_set());
    let before = fx.project.clone();
    let mut after = edited(&before, |p| {
        ripple_insert(p, fx.seq, 120, other, 30, 0);
    });
    follow(&before, &mut after);
    let set = set_in(&after, fx.seq);
    assert_eq!(frames(set.walls.stored.iter().map(|w| w.at)), vec![330]);
    assert_eq!(frames(set.walls.suppressed.iter().copied()), vec![210]);
    assert_eq!(frames(set.walls.candidates.iter().map(|c| c.at)), vec![380]);
    assert_eq!(
        spans(set),
        vec![
            (1, "part", 0, 180),
            (2, "part", 180, 430),
            (1, "region", 100, 230),
            (1, "speech", 100, 120),
            (3, "speech", 150, 230),
            (2, "music", 280, 330),
            (0, "area", 50, 120),
            (0, "area", 150, 190),
            (0, "analysed", 0, 120),
            (0, "analysed", 150, 430),
        ],
        "inserted frames were never analysed and carry no annotation or area"
    );
    assert_eq!(frames(set.box_tracks[0].keys.iter().map(|k| k.at)), vec![100, 180, 290]);
    let plan = &set.plans[0];
    assert_eq!(frames(plan.attention.iter().map(|m| m.at)), vec![180, 300]);
    let range = &plan.overwritten[0];
    assert_eq!((fr(range.from), fr(range.to)), (170, 210));
    assert_eq!(frames(range.positions.iter().map(|p| p.at)), vec![180], "the kept position follows its picture");
    assert_eq!(set.reference.as_ref().expect("the reference").frames, vec![100, 180, 290]);
    let walls_now = wall_frames_of(&after, fx.seq);
    assert!(walls_now.contains(&120) && walls_now.contains(&150), "every junction is a wall: no box interpolates across one");
    assert!(validate(&after, seq_of(&after, fx.seq)).is_empty());
}

#[test]
fn t_m33_data_follows_moved_content() {
    let mut fx = Fx::with_clips(&[(0, 200, 0), (200, 200, 10_000)]);
    let mut set = one_plan(vec![at25(1, 0, win(600.0, 540.0, 1080.0), Arrival::Cut), at25(2, 200, win(1200.0, 540.0, 1080.0), Arrival::Cut)]);
    set.walls.stored = vec![StoredWall { at: tk(300), origin: WallOrigin::Manual, score: None }];
    set.walls.suppressed = vec![tk(220)];
    set.annotations = vec![annotation(1, "speech", 150, 250)];
    set.next_annotation_id = 2;
    set.box_tracks = vec![dana(vec![key(260)])];
    set.plans[0].attention = vec![mark(50), mark(300)];
    fx.install(set);
    let before = fx.project.clone();
    // B moves in front of A: B [0, 200), A [200, 400)
    let mut after = edited(&before, |p| swap(p, fx.seq, 0));
    follow(&before, &mut after);
    let set = set_in(&after, fx.seq);
    assert_eq!(frames(set.walls.stored.iter().map(|w| w.at)), vec![100]);
    assert_eq!(frames(set.walls.suppressed.iter().copied()), vec![20]);
    let speech: Vec<(u32, i64, i64)> = set.annotations.iter().map(|a| (a.id, fr(a.from), fr(a.to))).collect();
    assert_eq!(speech, vec![(2, 0, 50), (1, 350, 400)], "the piece over A keeps the id; sorted by (from, id)");
    assert_eq!(set.next_annotation_id, 3);
    assert_eq!(frames(set.box_tracks[0].keys.iter().map(|k| k.at)), vec![60]);
    assert_eq!(frames(set.plans[0].attention.iter().map(|m| m.at)), vec![100, 250]);
    assert!(validate(&after, seq_of(&after, fx.seq)).is_empty());
}

#[test]
fn t_m32_attention_marks_follow_and_an_approval_reads_changed_when_its_part_moves() {
    let mut fx = Fx::with_clips(&[(0, 400, 0)]);
    let other = add_matte(&mut fx.project, "insert");
    let reviewed = |p: Position| Position { reviewed: true, ..p };
    let positions = vec![reviewed(at25(1, 0, win(600.0, 540.0, 1080.0), Arrival::Cut)), reviewed(at25(2, 100, win(900.0, 540.0, 1080.0), Arrival::Cut))];
    let mut set = one_plan(positions);
    set.parts = vec![part25(1, "P01", 100, 200)];
    set.next_part_id = 2;
    set.plans[0].attention = vec![mark(150)];
    fx.install(set);
    approve(&mut fx, 1, 1);
    assert_eq!(part_status(&fx.project, fx.seq(), 1, 1), Some(PartStatus::Approved));
    let approvals = plan_in(&fx.project, fx.seq, 1).part_approvals.clone();
    let before = fx.project.clone();
    let mut after = edited(&before, |p| {
        ripple_insert(p, fx.seq, 50, other, 20, 0);
    });
    follow(&before, &mut after);
    let plan = plan_in(&after, fx.seq, 1);
    assert_eq!(plan.part_approvals, approvals, "the hook never changes an approval");
    assert_eq!(frames(plan.attention.iter().map(|m| m.at)), vec![170]);
    assert_eq!(part_status(&after, seq_of(&after, fx.seq), 1, 1), Some(PartStatus::ChangedSinceApproval), "approvals bind to times");
}
