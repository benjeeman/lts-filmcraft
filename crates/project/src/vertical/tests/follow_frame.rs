//! T-M11, step 11 of the edit hook (spec section 4.8): the frame size or rate changed. The same aspect (HD to UHD) rescales
//! every window, focus point and rect exactly and re-rounds them; another aspect leaves the windows, flags every position
//! and raises the error `frame_changed`, which the plan keeps (it stores the frame its windows were made on) through
//! `markChecked`, a save and later edits, until a person repairs the plan or the frame returns; a new rate snaps positions
//! to the nearest frame, keeps every move's duration and flags `TimeApproximate` only where a position lands between
//! frames. Synthetic data only.

use filmcraft_time::FrameRate;

use super::fixtures::{Fx, at25, one_plan, part25, tk};
use super::hook_fixtures::*;
use super::support::{mv, same_bits, win};
use crate::vertical::*;
use crate::{ItemId, Project};

/// Doubles a window and rounds it onto the grid, as an HD to UHD rescale does.
fn twice(w: Window) -> Window {
    Window { cx: q(2.0 * w.cx), cy: q(2.0 * w.cy), h: q(2.0 * w.h) }
}

#[test]
fn t_m11_hd_to_uhd_rescales_every_window_and_rect_exactly() {
    let mut fx = Fx::with_clips(&[(0, 300, 0)]);
    let focused = Position { focus: Some([900.5, 400.25]), ..at25(2, 100, win(1000.5, 500.25, 810.0), mv(40, Curve::EaseInOut)) };
    let mut set = one_plan(vec![at25(1, 0, win(812.4, 540.0, 1080.0), Arrival::Cut), focused]);
    set.plans[0].starting_window = Some(win(960.0, 540.0, 1080.0));
    set.regions = vec![ProtectedRegion {
        id: 1,
        label: "logo".into(),
        from: tk(50),
        to: tk(150),
        rect: [100.5, 40.25, 180.0, 90.0],
        mode: RegionMode::AllOrNothing,
        origin: Origin::Manual,
        modified_by: Writer::Human,
    }];
    set.next_region_id = 2;
    set.box_tracks = vec![dana(vec![KeyBox { at: tk(10), rect: [700.5, 300.25, 200.0, 240.0], facing: None, gone: false }])];
    set.active_area = vec![ActiveArea { from: tk(0), to: tk(200), rect: [0.0, 132.0, 1920.0, 816.0], origin: Origin::Manual, modified_by: Writer::Human }];
    fx.install(set);
    let before = fx.project.clone();
    let mut after = edited(&before, |p| {
        let settings = &mut p.sequence_mut(fx.seq).expect("the sequence").settings;
        settings.width = 3840;
        settings.height = 2160;
    });
    let report = follow(&before, &mut after);
    let set = set_in(&after, fx.seq);
    let plan = &set.plans[0];
    assert!(same_bits(&plan.positions[0].window, &twice(win(812.4, 540.0, 1080.0))), "{:?}", plan.positions[0].window);
    assert!(same_bits(&plan.positions[1].window, &twice(win(1000.5, 500.25, 810.0))), "{:?}", plan.positions[1].window);
    assert_eq!(plan.positions[1].focus, Some([1801.0, 800.5]));
    assert!(same_bits(&plan.starting_window.expect("the starting window stays"), &twice(win(960.0, 540.0, 1080.0))));
    assert_eq!(set.regions[0].rect, [201.0, 80.5, 360.0, 180.0]);
    assert_eq!(set.box_tracks[0].keys[0].rect, [1401.0, 600.5, 400.0, 480.0]);
    assert_eq!(set.active_area[0].rect, [0.0, 264.0, 3840.0, 1632.0]);
    assert_eq!(frames_of(plan), vec![0, 100], "the frames stay");
    assert_eq!(plan.positions[1].arrival, mv(40, Curve::EaseInOut));
    assert!(plan.positions.iter().all(|p| p.flags.is_empty()), "an exact rescale flags nothing");
    assert!(report.sequences.iter().all(|s| s.issues.iter().all(|i| i.code != "frame_changed")));
    assert!(validate(&after, seq_of(&after, fx.seq)).is_empty());
}

#[test]
fn t_m11_another_aspect_leaves_the_windows_and_raises_frame_changed() {
    let mut fx = Fx::with_clips(&[(0, 300, 0)]);
    let windows = [win(600.0, 540.0, 1080.0), win(1000.0, 540.0, 1080.0)];
    fx.install(one_plan(vec![at25(1, 0, windows[0], Arrival::Cut), at25(2, 100, windows[1], Arrival::Cut)]));
    let before = fx.project.clone();
    let mut after = edited(&before, |p| p.sequence_mut(fx.seq).expect("the sequence").settings.width = 1440);
    let report = follow(&before, &mut after);
    let plan = plan_in(&after, fx.seq, 1);
    for (p, w) in plan.positions.iter().zip(windows) {
        assert!(same_bits(&p.window, &w), "#{} keeps its window", p.id);
        assert_eq!(p.flags, vec![Flag::PictureChanged], "#{}", p.id);
    }
    let issues: Vec<&Issue> = report.sequences.iter().flat_map(|s| &s.issues).filter(|i| i.code == "frame_changed").collect();
    assert_eq!(issues.len(), 1, "one per plan");
    assert_eq!((issues[0].level, issues[0].plan), (Level::Error, Some(1)), "it blocks vertical export until fixed");
    assert_eq!(plan_report(&report, fx.seq, 1).flagged, vec![1, 2]);
}

/// `w` planned as a person on sequence `seq` of `project` and applied to a copy, as the engine applies it.
fn written(project: &Project, seq: ItemId, w: &Write) -> (Planned, Project) {
    let planned = match plan_write(project, seq_of(project, seq), w, PERSON) {
        Ok(planned) => planned,
        Err(e) => panic!("refused {}: {} {}", e.code, e.message, e.detail),
    };
    let set = planned.set.clone();
    (planned, edited(project, |p| p.sequence_mut(seq).expect("the sequence").vertical = set))
}

/// How many `frame_changed` errors plan 1 of sequence `seq` of `project` has, as every read derives them.
fn frame_changed(project: &Project, seq: ItemId) -> usize {
    let found = issues(project, seq_of(project, seq), 1, 0, 300);
    found.iter().filter(|i| i.code == "frame_changed" && i.level == Level::Error).count()
}

#[test]
fn t_m11_frame_changed_holds_until_the_plan_is_repaired_or_the_frame_returns() {
    let mut fx = Fx::with_clips(&[(0, 300, 0)]);
    fx.install(one_plan(vec![at25(1, 0, win(600.0, 540.0, 1080.0), Arrival::Cut), at25(2, 100, win(1000.0, 540.0, 1080.0), Arrival::Cut)]));
    let before = fx.project.clone();
    let mut narrow = edited(&before, |p| p.sequence_mut(fx.seq).expect("the sequence").settings.width = 1440);
    follow(&before, &mut narrow);
    assert_eq!(plan_in(&narrow, fx.seq, 1).framed_on, Some([1920, 1080]), "the plan records the frame its windows were made on");
    assert_eq!(frame_changed(&narrow, fx.seq), 1, "an error of the plan: its vertical export is blocked");
    // a person clears the flags: the error stays
    let check = Write::MarkChecked(MarkChecked { plan: 1, positions: None, expect: None, expect_range: None });
    let (_, checked) = written(&narrow, fx.seq, &check);
    assert!(plan_in(&checked, fx.seq, 1).positions.iter().all(|p| p.flags.is_empty()));
    assert_eq!(frame_changed(&checked, fx.seq), 1, "markChecked clears flags, not the error");
    // saved and opened again: the error stays
    let saved = serde_json::to_string(set_in(&checked, fx.seq)).expect("the set saves");
    let opened: VerticalSet = serde_json::from_str(&saved).expect("the set opens");
    let reopened = edited(&checked, |p| p.sequence_mut(fx.seq).expect("the sequence").vertical = Some(Box::new(opened)));
    assert_eq!(frame_changed(&reopened, fx.seq), 1, "the file holds the frame the windows were made on");
    // a later 16:9 edit keeps it
    let mut rippled = edited(&checked, |p| ripple_delete(p, fx.seq, 200, 210));
    follow(&checked, &mut rippled);
    assert_eq!(frame_changed(&rippled, fx.seq), 1);
    // the frame back at 1920 wide: the windows belong to it again
    let mut back = edited(&checked, |p| p.sequence_mut(fx.seq).expect("the sequence").settings.width = 1920);
    follow(&checked, &mut back);
    assert_eq!((plan_in(&back, fx.seq, 1).framed_on, frame_changed(&back, fx.seq)), (None, 0));
    // or a person repairs the plan, and its windows belong to 1440x1080 from then on
    let (repair, repaired) = written(&checked, fx.seq, &Write::Repair(Repair { plan: 1 }));
    assert!(repair.changes.iter().any(|c| c.code == "frame_accepted"), "{:?}", repair.changes);
    assert_eq!((plan_in(&repaired, fx.seq, 1).framed_on, frame_changed(&repaired, fx.seq)), (None, 0));
}

#[test]
fn t_m11_a_rate_change_snaps_positions_and_keeps_move_durations() {
    let mut fx = Fx::with_clips(&[(0, 300, 0)]);
    let ramp = Curve::Ramp { r_in: 0.2, r_out: 0.2 };
    let mut set = one_plan(vec![
        at25(1, 0, win(600.0, 540.0, 1080.0), Arrival::Cut),
        at25(2, 40, win(800.0, 540.0, 1080.0), Arrival::Cut),
        at25(3, 100, win(1300.0, 600.0, 900.0), mv(30, ramp)),
    ]);
    set.walls.stored = vec![StoredWall { at: tk(150), origin: WallOrigin::Manual, score: None }];
    set.parts = vec![part25(1, "P01", 50, 250)];
    set.next_part_id = 2;
    fx.install(set);
    let before = fx.project.clone();
    let mut after = edited(&before, |p| p.sequence_mut(fx.seq).expect("the sequence").settings.frame_rate = FrameRate::FPS_50);
    follow(&before, &mut after);
    let fifty = FrameRate::FPS_50;
    let set = set_in(&after, fx.seq);
    let plan = &set.plans[0];
    assert_eq!(plan.positions.iter().map(|p| fifty.frame_at(p.at)).collect::<Vec<i64>>(), vec![0, 80, 200]);
    assert_eq!(plan.positions[2].arrival, mv(60, ramp), "the move keeps its 1.2 s");
    assert!(plan.positions.iter().all(|p| p.flags.is_empty()), "every 25p frame starts a 50p frame: nothing is approximate");
    assert_eq!(set.walls.stored.iter().map(|w| fifty.frame_at(w.at)).collect::<Vec<i64>>(), vec![300]);
    assert_eq!((fifty.frame_at(set.parts[0].from), fifty.frame_at(set.parts[0].to)), (100, 500));
    let (r0, r1) = (fx.r(), raster(seq_of(&after, fx.seq)));
    let old = plan_in(&before, fx.seq, 1);
    for f in 0..300 {
        let (a, b) = (window_at(old, &r0, r0.rate.tick_of(f)), window_at(plan, &r1, r1.rate.tick_of(2 * f)));
        assert!(same_bits(&a.window, &b.window), "25p frame {f} and 50p frame {}", 2 * f);
    }
    assert!(validate(&after, seq_of(&after, fx.seq)).is_empty());
}

#[test]
fn t_m11_a_rate_change_to_29_97_snaps_to_the_nearest_frames_and_flags_time_approximate() {
    let mut fx = Fx::with_clips(&[(0, 250, 0)]);
    fx.install(one_plan(vec![
        at25(1, 0, win(600.0, 540.0, 1080.0), Arrival::Cut),
        at25(2, 25, win(800.0, 540.0, 1080.0), Arrival::Cut),
        at25(3, 100, win(1300.0, 540.0, 1080.0), mv(50, Curve::Linear)),
    ]));
    let before = fx.project.clone();
    let mut after = edited(&before, |p| p.sequence_mut(fx.seq).expect("the sequence").settings.frame_rate = FrameRate::FPS_29_97);
    let report = follow(&before, &mut after);
    let ntsc = FrameRate::FPS_29_97;
    let plan = plan_in(&after, fx.seq, 1);
    // 1 s is 29.97 frames, 2 s 59.94 and 4 s 119.88: the nearest frames are 30, 60 and 120
    assert_eq!(plan.positions.iter().map(|p| ntsc.frame_at(p.at)).collect::<Vec<i64>>(), vec![0, 30, 120]);
    assert!(plan.positions.iter().all(|p| ntsc.tick_of(ntsc.frame_at(p.at)) == p.at), "every position sits on a frame start");
    assert_eq!(plan.positions[2].arrival, mv(60, Curve::Linear), "the move runs from 2 s to 4 s as before");
    assert!(plan.positions[0].flags.is_empty(), "frame 0 is exact");
    assert_eq!((plan.positions[1].flags.clone(), plan.positions[2].flags.clone()), (vec![Flag::TimeApproximate], vec![Flag::TimeApproximate]));
    assert_eq!(plan_report(&report, fx.seq, 1).flagged, vec![2, 3]);
    assert!(validate(&after, seq_of(&after, fx.seq)).is_empty());
}
