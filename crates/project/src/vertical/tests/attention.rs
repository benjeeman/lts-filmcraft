//! T-M32, the model half (spec sections 4.6 and 5.10): attention marks and approvals are stored, excluded from the range
//! (framing) and output digests, and a plan holds at most one approval per part. And the people's writes of M2: flagging
//! and clearing attention, review (Alt+R's one step: a `SAME` opener added when the shot has none, every position
//! reviewed, the shot's marks cleared) and lock.

use serde_json::json;

use super::fixtures::*;
use super::support::{mv, win};
use crate::vertical::*;

/// A wall at 100; a cut on 0 and a jump at 200 in the second shot, which has no opener; a part [100, 300).
fn shots() -> Fx {
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 900, 5_000)]);
    let mut set = one_plan(vec![at25(1, 0, win(700.0, 540.0, 1080.0), Arrival::Cut), at25(2, 200, win(1100.0, 540.0, 1080.0), Arrival::Cut)]);
    set.parts = vec![part25(1, "P01", 100, 300)];
    set.next_part_id = 2;
    fx.install(set);
    fx
}

fn flag(frame: i64, note: &str) -> Write {
    Write::FlagAttention(FlagAttention { plan: 1, frame, note: note.into(), by: "Dana Editor".into(), when: 1_790_000_000 })
}

#[test]
fn t_m32_attention_marks_are_stored_with_who_and_when() {
    let mut fx = shots();
    let planned = fx.apply(flag(150, "eyeline"));
    assert_eq!(fx.plan(1).attention, vec![AttentionMark { at: tk(150), by: "Dana Editor".into(), when: 1_790_000_000, note: "eyeline".into() }]);
    assert_eq!(planned.label, "Vertical: flag shot 2");
    assert!(!accepted(fx.write(flag(150, "eyeline"))).changed, "the same mark again changes nothing");
    fx.apply(flag(150, "eyeline, and the hair"));
    assert_eq!(fx.plan(1).attention.len(), 1, "one mark per frame: its note is updated");
    assert_eq!(fx.plan(1).attention[0].note, "eyeline, and the hair");
    fx.apply(flag(120, ""));
    let frames: Vec<i64> = fx.plan(1).attention.iter().map(|m| RATE.frame_at(m.at)).collect();
    assert_eq!(frames, vec![120, 150], "sorted by frame");
    let found = issues(&fx.project, fx.seq(), 1, 0, 1_000);
    let attention = with_code(&found, "attention");
    assert_eq!(attention.len(), 1, "one issue per shot with marks");
    assert_eq!((attention[0].level, attention[0].detail["shot"].clone(), attention[0].detail["frames"].clone()), (Level::Info, json!(2), json!([120, 150])));
    let clear = Write::ClearAttention(ClearAttention { plan: 1, frames: vec![150, 777] });
    fx.apply(clear.clone());
    assert_eq!(fx.plan(1).attention.len(), 1);
    assert!(!accepted(fx.write(clear)).changed, "clearing what is clear changes nothing");
}

#[test]
fn t_m32_marks_and_approvals_are_outside_the_range_and_output_digests() {
    let mut fx = shots();
    let part = fx.set().parts[0].clone();
    let digests = |fx: &Fx| (range_digest(&fx.project, fx.seq(), fx.plan(1), 100, 300), output_digest(&fx.project, fx.seq(), fx.plan(1), &part));
    let before = digests(&fx);
    assert!(!before.0.is_empty() && !before.1.is_empty());
    fx.plan_mut(1).attention = vec![AttentionMark { at: tk(150), by: "Dana Editor".into(), when: 1, note: "x".into() }];
    fx.plan_mut(1).part_approvals = vec![PartApproval {
        part: 1,
        framing_digest: before.0.clone(),
        output_digest: before.1.clone(),
        by: "Dana Editor".into(),
        when: 1,
        note: String::new(),
    }];
    assert_eq!(digests(&fx), before);
    // a position changes the range digest
    fx.plan_mut(1).positions[1].window.cx = 1_000.0;
    assert_ne!(digests(&fx).0, before.0);
}

#[test]
fn t_m32_a_plan_holds_at_most_one_approval_per_part() {
    let mut fx = shots();
    // a second plan
    let mut second = VerticalPlan::new(2, "4:5", OutputSize { width: 1080, height: 1350 });
    second.positions = vec![at25(1, 100, win(960.0, 540.0, 1080.0), Arrival::Cut)];
    second.next_position_id = 2;
    fx.set_mut().plans.push(second);
    fx.set_mut().next_plan_id = 3;
    for plan in [1, 2] {
        for p in &mut fx.plan_mut(plan).positions {
            p.reviewed = true;
        }
    }
    // plan 1's shot from 100 has no opener: Alt+R adds a SAME opener, reviewed
    fx.apply(Write::SetReview(SetReview { plan: 1, shot_frame: Some(150), reviewed: true, ..Default::default() }));
    let approve = |plan: u32, when: i64, note: &str| Write::ApprovePart(ApprovePart { plan, part: 1, note: note.into(), by: "Dana Editor".into(), when });
    fx.apply(approve(1, 1, ""));
    let again = accepted(fx.write(approve(1, 2, "")));
    assert!(!again.changed, "the same approval again (same digests, name and note) is a retry: nothing changes");
    fx.apply(approve(1, 3, "second look"));
    fx.apply(approve(2, 4, ""));
    assert_eq!(fx.plan(1).part_approvals.len(), 1);
    let kept = &fx.plan(1).part_approvals[0];
    assert_eq!((kept.when, kept.note.as_str()), (3, "second look"), "approving again replaces the plan's approval");
    assert_eq!(fx.plan(2).part_approvals.len(), 1, "each plan has its own");
    assert_eq!(part_status(&fx.project, fx.seq(), 2, 1), Some(PartStatus::Approved));
}

#[test]
fn set_review_on_a_shot_adds_a_same_opener_marks_every_position_and_clears_its_marks() {
    let mut fx = shots();
    fx.plan_mut(1).attention = vec![AttentionMark { at: tk(150), by: "Dana Editor".into(), when: 1, note: "x".into() }];
    let planned = fx.apply(Write::SetReview(SetReview { plan: 1, shot_frame: Some(150), reviewed: true, ..Default::default() }));
    assert_eq!(planned.label, "Vertical: approve shot 2");
    assert_eq!(fx.frames(1), vec![0, 100, 200]);
    let opener = fx.plan(1).positions[1].clone();
    assert_eq!((opener.window, opener.arrival, opener.reviewed, opener.origin), (win(700.0, 540.0, 1080.0), Arrival::Cut, true, Origin::Manual));
    assert!(fx.position(1, 2).reviewed);
    assert!(!fx.position(1, 1).reviewed, "the first shot is not touched");
    assert!(fx.plan(1).attention.is_empty(), "the shot's marks are cleared");
    assert!(planned.changed_frames.is_empty(), "a SAME opener changes no frame");
    // unreviewing by ids
    fx.apply(Write::SetReview(SetReview { plan: 1, positions: vec![2], reviewed: false, ..Default::default() }));
    assert!(!fx.position(1, 2).reviewed);
    assert!(!accepted(fx.write(Write::SetReview(SetReview { plan: 1, positions: vec![2], reviewed: false, ..Default::default() }))).changed);
}

#[test]
fn approving_the_first_shot_cuts_a_move_its_new_opener_leaves_no_room() {
    // a 20-frame move on the first position, at 10, has no effect; Alt+R's opener at 0 would leave it 10 frames
    let mut fx = Fx::new(1_000);
    fx.install(one_plan(vec![at25(1, 10, win(700.0, 540.0, 1080.0), mv(20, Curve::Linear))]));
    let planned = fx.apply(Write::SetReview(SetReview { plan: 1, shot_frame: Some(10), reviewed: true, ..Default::default() }));
    assert_eq!(planned.label, "Vertical: approve shot 1");
    assert_eq!(fx.frames(1), vec![0, 10]);
    assert_eq!(fx.position(1, 1).arrival, Arrival::Cut);
    assert!(fx.plan(1).positions.iter().all(|p| p.reviewed));
    assert_eq!(planned.changes.iter().map(|c| c.code).collect::<Vec<_>>(), vec!["move_cut"]);
    assert!(planned.changed_frames.is_empty(), "the SAME opener and the cut to the same window change no frame");
}

#[test]
fn set_lock_locks_and_unlocks_and_a_locked_position_still_takes_review() {
    let mut fx = shots();
    let planned = fx.apply(Write::SetLock(SetLock { plan: 1, positions: vec![2], locked: true }));
    assert_eq!(planned.label, "Vertical: lock #2");
    assert!(fx.position(1, 2).locked);
    fx.apply(Write::SetReview(SetReview { plan: 1, positions: vec![2], reviewed: true, ..Default::default() }));
    assert!(fx.position(1, 2).reviewed, "review is a person's act on a locked position too");
    assert_eq!(fx.refuse(set_at(200, PositionFields { cx: Some(1_000.0), ..Default::default() })).code, "locked");
    fx.apply(Write::SetLock(SetLock { plan: 1, positions: vec![2], locked: false }));
    assert!(!fx.position(1, 2).locked);
    assert_eq!(fx.refuse(Write::SetLock(SetLock { plan: 1, positions: vec![9], locked: true })).code, "not_found");
}
