//! T-M35 extended to `rangeDigest` and `effectiveWallsDigest` (spec section 2.6). The range digest of `[from, to)` covers
//! the positions with a frame in it (the fields whose change is a change, spec section 7.5: not `reviewed`, `locked`,
//! flags, origin, `modifiedBy` or `proposed`), the position governing `from` (`window_at`'s position: inside a move the one
//! it departs from; its id, window, layout and arrival only), the effective walls in `[from, to]` with their sources (not
//! their scores: re-measuring a cut does not reframe anything), and the parts (their ids, names and ranges) and regions
//! overlapping it; never the counters, attention marks, approvals, annotations, box tracks, active areas, plan settings, or
//! anything outside the range. The effective-walls digest covers the effective walls with every source, not their scores:
//! it changes with a 16:9 edit that moves an edit point. Both are pure functions of their content, printed `fnv1a64:` and
//! 16 hex digits.

use super::fixtures::*;
use super::support::{digest_ok, mv, win};
use crate::Label;
use crate::vertical::*;

/// Edit walls at 100 and 400; positions at 50, 150 (a move from 120), 250 and 450; a stored wall at 300, a part
/// [120, 380) and a region over [0, 1000).
fn ranged() -> Fx {
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 300, 5_000), (400, 600, 9_000)]);
    let mut set = one_plan(vec![
        Position { label: "Dana".into(), ..at25(1, 50, win(700.0, 540.0, 1080.0), Arrival::Cut) },
        at25(2, 150, win(1000.0, 540.0, 1080.0), mv(30, Curve::EaseInOut)),
        at25(3, 250, win(900.0, 540.0, 1080.0), Arrival::Cut),
        at25(4, 450, win(800.0, 540.0, 1080.0), Arrival::Cut),
    ]);
    set.walls.stored = vec![stored(300, WallOrigin::Manual, None)];
    set.parts = vec![part25(1, "P01", 120, 380)];
    set.regions = vec![ProtectedRegion {
        id: 1,
        label: "logo".into(),
        from: tk(0),
        to: tk(1_000),
        rect: [1700.0, 40.0, 180.0, 90.0],
        mode: RegionMode::AllOrNothing,
        origin: Origin::Manual,
        modified_by: Writer::Human,
    }];
    fx.install(set);
    fx
}

/// The range digest of [130, 350) and the effective-walls digest.
fn digests(fx: &Fx) -> (String, String) {
    (range_digest(&fx.project, fx.seq(), fx.plan(1), 130, 350), effective_walls_digest(&fx.project, fx.seq()))
}

/// Which of the two digests change when `edit` changes the ranged fixture (each a well-formed digest before and after).
fn changed(edit: impl FnOnce(&mut Fx)) -> (bool, bool) {
    let mut fx = ranged();
    let before = digests(&fx);
    edit(&mut fx);
    let after = digests(&fx);
    for d in [&before.0, &before.1, &after.0, &after.1] {
        assert!(digest_ok(d), "{d}");
    }
    (after.0 != before.0, after.1 != before.1)
}

#[test]
fn t_m35_range_and_effective_walls_digests_print_fnv1a64_and_are_pure() {
    let (range, walls) = digests(&ranged());
    assert!(digest_ok(&range), "{range}");
    assert!(digest_ok(&walls), "{walls}");
    assert_eq!(digests(&ranged()), (range, walls), "equal content, equal digests");
}

#[test]
fn t_m35_the_range_digest_covers_the_range() {
    assert_eq!(changed(|fx| fx.plan_mut(1).positions[1].window.cx = 1_001.0), (true, false), "a position in the range");
    assert_eq!(changed(|fx| fx.plan_mut(1).positions[2].note = "why".into()), (true, false), "a position's metadata in the range");
    assert_eq!(changed(|fx| fx.plan_mut(1).positions[0].window.cx = 701.0), (true, false), "the position a move through `from` departs from");
    assert_eq!(changed(|fx| fx.plan_mut(1).positions[0].label = "Noa".into()), (false, false), "the governing position's label is not in it");
    assert_eq!(changed(|fx| fx.plan_mut(1).positions[3].window.cx = 801.0), (false, false), "a position after the range");
    assert_eq!(changed(|fx| fx.set_mut().regions[0].rect[0] = 1_701.0), (true, false), "a region overlapping it");
    assert_eq!(changed(|fx| fx.set_mut().parts[0].name = "P02".into()), (true, false), "a part overlapping it");
}

#[test]
fn t_m35_the_range_digest_leaves_out_what_is_not_framing() {
    assert_eq!(changed(|fx| fx.plan_mut(1).next_position_id = 99), (false, false), "counters");
    assert_eq!(changed(|fx| fx.plan_mut(1).attention.push(AttentionMark { at: tk(200), by: "x".into(), when: 1, note: String::new() })), (false, false));
    assert_eq!(
        changed(|fx| {
            fx.plan_mut(1).part_approvals.push(PartApproval {
                part: 1,
                framing_digest: String::new(),
                output_digest: String::new(),
                by: "x".into(),
                when: 1,
                note: String::new(),
            })
        }),
        (false, false),
        "approvals"
    );
    assert_eq!(changed(|fx| fx.plan_mut(1).captions.max_lines = 3), (false, false), "plan settings");
    assert_eq!(changed(|fx| fx.set_mut().parts[0].meta.title = "Dana walks".into()), (false, false), "a part's metadata is the output digest's");
    assert_eq!(changed(|fx| fx.set_mut().parts[0].audio = Some(PartAudio { fade_in_frames: 0, fade_out_frames: 0 })), (false, false), "and so are its fades");
    assert_eq!(
        changed(|fx| {
            fx.set_mut().annotations.push(Annotation {
                id: 1,
                kind: "speech".into(),
                from: tk(140),
                to: tk(160),
                label: String::new(),
                score: None,
                origin: Origin::Agent,
                modified_by: Writer::Agent,
            })
        }),
        (false, false),
        "annotations"
    );
    assert_eq!(
        changed(|fx| {
            let area = ActiveArea { from: tk(0), to: tk(500), rect: [0.0, 132.0, 1920.0, 816.0], origin: Origin::Manual, modified_by: Writer::Human };
            fx.set_mut().active_area.push(area);
        }),
        (false, false),
        "active areas"
    );
}

/// The ranged fixture's position 2 (frame 150, a move from 120): inside [130, 350).
fn walk(fx: &mut Fx) -> &mut Position {
    &mut fx.plan_mut(1).positions[1]
}

#[test]
fn t_m35_the_range_digest_holds_the_fields_whose_change_is_a_change() {
    let proposal = Proposal { at: tk(150), window: win(990.0, 540.0, 1080.0), layout: Layout::Fill, arrival: Arrival::Cut };
    assert_eq!(changed(|fx| walk(fx).locked = true), (false, false), "locked");
    assert_eq!(changed(|fx| walk(fx).reviewed = true), (false, false), "reviewed");
    assert_eq!(changed(|fx| walk(fx).flags = vec![Flag::MoveShortened]), (false, false), "flags");
    assert_eq!(changed(|fx| walk(fx).origin = Origin::Agent), (false, false), "origin");
    assert_eq!(changed(|fx| walk(fx).modified_by = Writer::Agent), (false, false), "modifiedBy");
    assert_eq!(changed(|fx| walk(fx).proposed = Some(proposal)), (false, false), "proposed");
    assert_eq!(changed(|fx| walk(fx).caption_anchor = Some(CaptionAnchor::Top)), (true, false), "caption anchor");
    assert_eq!(changed(|fx| walk(fx).label = "Dana".into()), (true, false), "label");
    assert_eq!(changed(|fx| walk(fx).color = Some(Label::Rose)), (true, false), "colour");
    assert_eq!(changed(|fx| walk(fx).tags = vec!["walk".into()]), (true, false), "tags");
    assert_eq!(changed(|fx| walk(fx).subjects = vec!["char:dana".into()]), (true, false), "subjects");
    assert_eq!(changed(|fx| walk(fx).focus = Some([1_000.0, 500.0])), (true, false), "focus");
    assert_eq!(changed(|fx| walk(fx).confidence = Some(0.5)), (true, false), "confidence");
    assert_eq!(changed(|fx| walk(fx).risk = "fast".into()), (true, false), "risk");
}

#[test]
fn t_m35_locking_reviewing_and_marking_checked_keep_a_part_approved() {
    let mut fx = Fx::new(1_000);
    let reviewed = |p: Position| Position { reviewed: true, ..p };
    let (first, second) = (at25(1, 100, win(700.0, 540.0, 1080.0), Arrival::Cut), at25(2, 200, win(1100.0, 540.0, 1080.0), Arrival::Cut));
    let mut set = one_plan(vec![reviewed(first), reviewed(second)]);
    set.parts = vec![part25(1, "P01", 100, 300)];
    set.next_part_id = 2;
    fx.install(set);
    fx.apply(Write::ApprovePart(ApprovePart { plan: 1, part: 1, note: String::new(), by: "Dana Editor".into(), when: 1_790_000_000 }));
    let status = |fx: &Fx| part_status(&fx.project, fx.seq(), 1, 1);
    let framing = |fx: &Fx| range_digest(&fx.project, fx.seq(), fx.plan(1), 100, 300);
    let approved = framing(&fx);
    assert_eq!(status(&fx), Some(PartStatus::Approved));
    let lock = fx.apply(Write::SetLock(SetLock { plan: 1, positions: vec![2], locked: true }));
    assert!(with_code(&lock.issues, "part_changed_since_approval").is_empty(), "{:?}", lock.issues);
    assert_eq!(status(&fx), Some(PartStatus::Approved), "a lock is no framing change");
    fx.apply(Write::SetReview(SetReview { plan: 1, positions: vec![2], reviewed: false, ..Default::default() }));
    assert_eq!(status(&fx), Some(PartStatus::Approved), "nor is a review state");
    fx.plan_mut(1).positions[0].flags = vec![Flag::PictureChanged];
    fx.apply(Write::MarkChecked(MarkChecked { plan: 1, positions: None, ..Default::default() }));
    assert_eq!(status(&fx), Some(PartStatus::Approved), "nor are flags");
    assert_eq!(framing(&fx), approved);
    // a framing change is one
    fx.apply(Write::SetLock(SetLock { plan: 1, positions: vec![2], locked: false }));
    fx.apply(set_at(200, PositionFields { cx: Some(1_000.0), ..Default::default() }));
    assert_eq!(status(&fx), Some(PartStatus::ChangedSinceApproval));
}

#[test]
fn t_m35_the_effective_walls_digest_follows_every_source() {
    assert_eq!(changed(|fx| fx.set_mut().walls.stored.push(stored(500, WallOrigin::Agent, Some(0.5)))), (false, true), "a stored wall outside the range");
    assert_eq!(changed(|fx| fx.set_mut().walls.stored.insert(0, stored(200, WallOrigin::Agent, None))), (true, true), "a stored wall inside the range");
    assert_eq!(changed(|fx| fx.set_mut().walls.stored[0].score = Some(0.25)), (false, false), "a score is the stored walls digest's: it moves no wall");
    assert_eq!(changed(|fx| fx.set_mut().walls.suppressed.push(tk(400))), (false, true), "suppressing an edit wall outside the range");
    assert_eq!(changed(|fx| fx.set_mut().walls.candidates.push(Candidate { at: tk(200), score: 0.1 })), (false, false), "a candidate is no wall");
    assert_eq!(changed(|fx| fx.set_mut().walls.analysed.push(FrameRange { from: tk(0), to: tk(500) })), (false, false), "coverage is no wall");
    assert_eq!(changed(|fx| fx.set_mut().walls.use_edit_points = false), (false, true), "the edit walls at 100 and 400 go, both outside [130, 350]");
    assert_eq!(changed(|fx| fx.set_mut().parts[0].to = tk(390)), (true, true), "a part boundary is a wall");
    // a 16:9 edit: the third clip starts later
    assert_eq!(
        changed(|fx| {
            let clip = fx.seq().video_tracks[0].items[2].id;
            fx.item_mut(clip).start = tk(410);
        }),
        (false, true),
        "the edit point at 400 moves to 410"
    );
}
