//! T-M11, hidden picture (spec section 4.8, step 2; lane L1c round 3): picture that an overlay, a switched-off track or
//! clip, or another multicam angle hides is not removed content. Its positions stay on their frames, flagged
//! `PictureChanged` (what the frames show on top changed), and when the picture shows again they are still there, so a
//! person checking a layer with the track's eye loses no framing. Synthetic data only.

use filmcraft_time::TimeRange;

use super::fixtures::{Fx, RATE, at25, one_plan, tk};
use super::hook_fixtures::*;
use super::support::{mv, win};
use crate::vertical::*;
use crate::{ClipId, ItemId, MulticamSel, Project, TrackKind};

/// A full-height window at `cx`.
fn full(cx: f64) -> Window {
    win(cx, 540.0, 1080.0)
}

/// The map of an edit that moves no frame, on a timeline of `end` frames.
fn identity(end: i64) -> Vec<Option<i64>> {
    (0..end).map(Some).collect()
}

/// The ids of a plan's positions flagged `PictureChanged`, in frame order.
fn picture_changed(plan: &VerticalPlan) -> Vec<u32> {
    plan.positions.iter().filter(|p| p.flags.contains(&Flag::PictureChanged)).map(|p| p.id).collect()
}

/// Puts a clip of `media`, `frames` long from source frame 0, on video track 1 of sequence `seq` at frame `at`: an
/// overlay (a still, a logo, a picture in picture). Returns it.
fn overlay(project: &mut Project, seq: ItemId, media: ItemId, at: i64, frames: i64) -> ClipId {
    let item = project.make_track_item(media, TrackKind::Video, tk(at), TimeRange::new(tk(0), tk(frames)), RATE).expect("the media exists");
    let id = item.id;
    video_mut(project, seq, 1).items.push(item);
    id
}

#[test]
fn t_m11_an_overlay_hides_picture_without_removing_its_positions() {
    let mut fx = Fx::with_clips(&[(0, 300, 0)]);
    let logo = add_matte(&mut fx.project, "logo");
    fx.install(one_plan(vec![
        at25(1, 0, full(600.0), Arrival::Cut),
        at25(2, 100, full(1000.0), Arrival::Cut),
        at25(3, 200, full(1300.0), mv(40, Curve::Linear)),
    ]));
    let before = fx.project.clone();
    let mut after = edited(&before, |p| {
        overlay(p, fx.seq, logo, 50, 200);
    });
    let report = follow(&before, &mut after);
    let (old, new) = (plan_in(&before, fx.seq, 1), plan_in(&after, fx.seq, 1));
    assert_eq!(frames_of(new), frames_of(old), "the picture under the overlay is still there: nothing moves, nothing goes");
    assert_eq!(picture_changed(new), vec![1, 2, 3], "the positions whose frames the overlay covers are flagged");
    assert_eq!(find(new, 3).expect("#3").arrival, mv(40, Curve::Linear));
    assert_follows(&before, &after, fx.seq, 1, &identity(300), "an overlay");
    let pr = plan_report(&report, fx.seq, 1);
    assert_eq!((pr.removed, pr.moved, pr.flagged), (vec![], vec![], vec![1, 2, 3]));
    assert!(validate(&after, seq_of(&after, fx.seq)).is_empty());
    // the overlay taken away again: the same positions
    let mut back = edited(&after, |p| video_mut(p, fx.seq, 1).items.clear());
    let report = follow(&after, &mut back);
    assert_eq!(ids_of(plan_in(&back, fx.seq, 1)), vec![1, 2, 3]);
    assert!(plan_report(&report, fx.seq, 1).removed.is_empty());
    assert_follows(&before, &back, fx.seq, 1, &identity(300), "the overlay taken away");
}

/// Clips A [0, 100) and B [100, 300); a move into #2 inside A, #3 opens B, a move into #4 inside B.
fn two_clips() -> Fx {
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 200, 10_000)]);
    fx.install(one_plan(vec![
        at25(1, 0, full(600.0), Arrival::Cut),
        at25(2, 50, full(800.0), mv(20, Curve::Linear)),
        at25(3, 100, full(1200.0), Arrival::Cut),
        at25(4, 200, full(1000.0), mv(40, Curve::EaseInOut)),
    ]));
    fx
}

#[test]
fn t_m11_a_track_switched_off_and_on_keeps_its_positions() {
    let fx = two_clips();
    let before = fx.project.clone();
    let mut off = edited(&before, |p| video_mut(p, fx.seq, 0).enabled = false);
    let report = follow(&before, &mut off);
    let hidden = plan_in(&off, fx.seq, 1);
    assert_eq!(frames_of(hidden), vec![0, 50, 100, 200], "a switched-off track's picture is hidden, not removed");
    assert_eq!(picture_changed(hidden), vec![1, 2, 3, 4], "the frames show black now");
    assert!(plan_report(&report, fx.seq, 1).removed.is_empty());
    assert_follows(&before, &off, fx.seq, 1, &identity(300), "the track switched off");
    // on again: the same positions with the same windows and moves
    let mut on = edited(&off, |p| video_mut(p, fx.seq, 0).enabled = true);
    let report = follow(&off, &mut on);
    assert!(plan_report(&report, fx.seq, 1).removed.is_empty());
    assert_eq!(ids_of(plan_in(&on, fx.seq, 1)), vec![1, 2, 3, 4]);
    assert_follows(&before, &on, fx.seq, 1, &identity(300), "the track switched on again");
    assert!(validate(&on, seq_of(&on, fx.seq)).is_empty());
}

#[test]
fn t_m11_a_clip_switched_off_and_on_keeps_its_positions() {
    let fx = two_clips();
    let b = clip_ids(&fx.project, fx.seq)[1];
    let before = fx.project.clone();
    let mut off = edited(&before, |p| clip_mut(p, fx.seq, b).enabled = false);
    follow(&before, &mut off);
    let hidden = plan_in(&off, fx.seq, 1);
    assert_eq!(frames_of(hidden), vec![0, 50, 100, 200], "B's positions stay with its hidden picture");
    assert_eq!(picture_changed(hidden), vec![3, 4], "only the positions over B's frames");
    let mut on = edited(&off, |p| clip_mut(p, fx.seq, b).enabled = true);
    follow(&off, &mut on);
    assert_follows(&before, &on, fx.seq, 1, &identity(300), "the clip switched on again");
    assert!(wall_frames_of(&on, fx.seq).contains(&100), "B's edit point is a wall again");
    assert!(validate(&on, seq_of(&on, fx.seq)).is_empty());
}

/// A multicam source sequence: camera A (the matte) on its first video track and camera B (another matte) on its second,
/// 300 frames each; the episode shows it from frame 0 as a multicam clip on angle 0.
fn multicam() -> Fx {
    let mut fx = Fx::with_clips(&[]);
    let camera_b = add_matte(&mut fx.project, "camera B");
    let settings = crate::SequenceSettings { frame_rate: RATE, ..Default::default() };
    let cameras = fx.project.new_sequence("Cameras", settings, 2, 0, None);
    let media = fx.media;
    let a = fx.make_item(media, tk(0), 300, 0);
    let b = fx.make_item(camera_b, tk(0), 300, 0);
    video_mut(&mut fx.project, cameras, 0).items.push(a);
    video_mut(&mut fx.project, cameras, 1).items.push(b);
    let mut clip = fx.make_item(cameras, tk(0), 300, 0);
    clip.multicam = Some(MulticamSel { enabled: true, angle: 0 });
    fx.put(0, clip);
    fx
}

#[test]
fn t_m11_a_multicam_angle_switch_keeps_the_positions_of_the_angle_it_hides() {
    let mut fx = multicam();
    fx.install(one_plan(vec![
        at25(1, 0, full(600.0), Arrival::Cut),
        at25(2, 100, full(1000.0), Arrival::Cut),
        at25(3, 200, full(1200.0), mv(30, Curve::Linear)),
    ]));
    let before = fx.project.clone();
    // a cut at 150 and camera B from there on
    let mut after = edited(&before, |p| {
        let right = razor(p, fx.seq, 150);
        clip_mut(p, fx.seq, right).multicam = Some(MulticamSel { enabled: true, angle: 1 });
    });
    let report = follow(&before, &mut after);
    let new = plan_in(&after, fx.seq, 1);
    assert_eq!(frames_of(new), vec![0, 100, 200], "camera A's frames are hidden, not removed");
    assert_eq!(picture_changed(new), vec![2, 3], "the positions whose frames now show camera B");
    assert!(wall_frames_of(&after, fx.seq).contains(&150), "the angle change is a cut");
    assert!(plan_report(&report, fx.seq, 1).removed.is_empty());
    assert_follows(&before, &after, fx.seq, 1, &identity(300), "an angle switch");
    assert!(validate(&after, seq_of(&after, fx.seq)).is_empty());
    // camera A again: the same positions
    let mut back = edited(&after, |p| {
        let right = clip_ids(p, fx.seq)[1];
        clip_mut(p, fx.seq, right).multicam = Some(MulticamSel { enabled: true, angle: 0 });
    });
    follow(&after, &mut back);
    assert_eq!(ids_of(plan_in(&back, fx.seq, 1)), vec![1, 2, 3]);
    assert_follows(&before, &back, fx.seq, 1, &identity(300), "the angle switched back");
}
