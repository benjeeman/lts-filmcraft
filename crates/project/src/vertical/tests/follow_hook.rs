//! T-M11, the edit hook at model level (spec section 4.8): hand-built before and after projects. Every surviving content
//! frame keeps its window within two split bounds (lane L1a's corrected bound, so not the spec's flat 2 delta), extended
//! heads and retimed content excepted and flagged; openers follow their clip; new content inherits; resumed content gets an
//! opener; suppressions follow their picture and one on removed content is dropped; a junction inside a nest gets a
//! `reanchored` wall; the worked example of section 4.8; a person's edit removes, shortens or remaps locked positions and
//! reports them (I16); and the before project stays untouched. Synthetic data only.

use serde_json::json;

use super::fixtures::{Fx, at25, one_plan, tk};
use super::hook_fixtures::*;
use super::support::{OUT, Rng, every_curve, hd, mixed_plan, mv, mv_split, random_window, same_bits, win};
use crate::vertical::*;
use crate::{ItemId, ParamValue, Project};

/// A full-height window at `cx`.
fn full(cx: f64) -> Window {
    win(cx, 540.0, 1080.0)
}

/// The window the hook stores where it splits a move at `frame` of `plan`: the exact window there, with directed rounding.
fn split_window(plan: &VerticalPlan, r: &Raster, frame: i64) -> Window {
    clamp_window(window_at(plan, r, r.rate.tick_of(frame)).window, r, OUT)
}

#[test]
fn t_m11_the_worked_example_of_section_4_8() {
    let mut fx = Fx::with_clips(&[(0, 400, 0)]);
    let (pi, pj) = (full(812.4), full(960.0));
    fx.install(one_plan(vec![at25(1, 100, pi, Arrival::Cut), at25(2, 150, pj, mv(50, Curve::EaseInOut))]));
    let before = fx.project.clone();
    let mut after = edited(&before, |p| ripple_delete(p, fx.seq, 120, 130));
    let report = follow(&before, &mut after);
    let (r, old, new) = (fx.r(), plan_in(&before, fx.seq, 1), plan_in(&after, fx.seq, 1));
    // exact positions are added at 119 and 130; 119 -> 119, 130 -> 120, 150 -> 140
    assert_eq!(frames_of(new), vec![100, 119, 120, 140]);
    assert_eq!(ids_of(new), vec![1, 3, 4, 2], "the added positions take the next ids, in frame order");
    let (left, right, pj_new) = (find(new, 3).expect("#3"), find(new, 4).expect("#4"), find(new, 2).expect("P_j"));
    assert!(same_bits(&left.window, &split_window(old, &r, 119)), "{:?}", left.window);
    assert_eq!(left.arrival, mv_split(19, Curve::EaseInOut, 0, 50), "P_i -> 119 is part 0-19 of the 50-frame ease");
    assert!(same_bits(&right.window, &split_window(old, &r, 130)), "{:?}", right.window);
    assert_eq!(right.arrival, Arrival::Cut, "119 -> 120 is a cut, where the picture also cuts");
    assert_eq!(pj_new.arrival, mv_split(20, Curve::EaseInOut, 30, 50), "120 -> 140 is part 30-50 of the ease");
    assert!(same_bits(&pj_new.window, &pj));
    assert_eq!(find(new, 1), find(old, 1), "P_i is untouched");
    for p in [left, right] {
        assert_eq!((p.origin, p.reviewed, p.locked), (Origin::Reanchored, false, false), "#{}", p.id);
    }
    assert_eq!(new.next_position_id, 5);
    assert!(wall_frames_of(&after, fx.seq).contains(&120), "the cut is a wall: the ripple left an edit point there");
    assert!(stored_walls(&after, fx.seq, WallOrigin::Reanchored).is_empty(), "an edit point is a wall already");
    assert_follows(&before, &after, fx.seq, 1, &deleted(400, 120, 130), "the worked example");
    assert!(validate(&after, seq_of(&after, fx.seq)).is_empty());
    let pr = plan_report(&report, fx.seq, 1);
    assert_eq!((pr.moved, pr.removed, pr.flagged, pr.added, pr.locked), (vec![2], vec![], vec![], vec![3, 4], vec![]));
    assert_eq!(report.label_suffix().as_deref(), Some("· vertical: 1 moved, 2 added"));
}

/// A random timeline: 1 to 4 back-to-back clips of the matte, 60 to 200 frames each, each from its own region of the
/// media, as `(start, frames, source)`.
fn random_clips(rng: &mut Rng) -> Vec<(i64, i64, i64)> {
    let mut clips = Vec::new();
    let mut start = 0;
    for k in 0..rng.int(1, 4) {
        let frames = rng.int(60, 200);
        clips.push((start, frames, 10_000 * (k + 1)));
        start += frames;
    }
    clips
}

/// Random positions over `clips`: 0 to 4 per clip (half the time one opens the clip), the first of each clip a cut, the
/// later ones cuts or moves that fit, of every curve; some reviewed, some locked, each labelled.
fn random_positions(rng: &mut Rng, r: &Raster, clips: &[(i64, i64, i64)]) -> Vec<Position> {
    let curves = every_curve();
    let mut positions = Vec::new();
    let mut id = 1;
    for &(start, frames, _) in clips {
        let mut picked: Vec<i64> = (0..rng.int(0, 4)).map(|_| start + rng.int(0, frames - 1)).collect();
        if rng.chance(0.5) {
            picked.push(start);
        }
        picked.sort_unstable();
        picked.dedup();
        let mut previous: Option<i64> = None;
        for f in picked {
            let window = random_window(rng, r, OUT);
            let arrival = match previous {
                Some(last) if f - last >= 2 && rng.chance(0.7) => mv(rng.int(2, f - last) as u32, curves[rng.int(0, 8) as usize]),
                _ => Arrival::Cut,
            };
            let mut p = at25(id, f, window, arrival);
            p.reviewed = rng.chance(0.3);
            p.locked = rng.chance(0.2);
            p.label = format!("p{id}");
            positions.push(p);
            previous = Some(f);
            id += 1;
        }
    }
    positions
}

/// Half the time, a frame strictly inside a random move (when there is one).
fn inside_a_move(rng: &mut Rng, moves: &[(i64, i64)]) -> Option<i64> {
    if moves.is_empty() || rng.chance(0.5) {
        return None;
    }
    let (s, f) = moves[rng.int(0, moves.len() as i64 - 1) as usize];
    (f - s >= 2).then(|| rng.int(s + 1, f - 1))
}

/// A range to ripple-delete, `[x, y)` with `x < y <= end`: half the time it starts inside a move.
fn cut_range(rng: &mut Rng, moves: &[(i64, i64)], end: i64) -> (i64, i64) {
    let x = inside_a_move(rng, moves).unwrap_or_else(|| rng.int(0, end - 1));
    (x, (x + rng.int(1, 40)).min(end))
}

/// The frames of `frames` inside `(0, end)`, sorted and unique.
fn inside(mut frames: Vec<i64>, end: i64) -> Vec<i64> {
    frames.retain(|&f| f > 0 && f < end);
    frames.sort_unstable();
    frames.dedup();
    frames
}

/// One random 16:9 edit: the edited copy, the oracle map, the new frames where the picture now cuts at a junction (edit
/// points: walls), and a description.
struct Case {
    after: Project,
    map: Vec<Option<i64>>,
    cuts: Vec<i64>,
    what: String,
}

/// Ripple deletes and inserts (half the time inside a move), tail trims both ways, head trims, a swap of two clips, a
/// replaced clip, or a delete and an insert in one edit.
fn random_edit(rng: &mut Rng, fx: &Fx, clips: &[(i64, i64, i64)], other: ItemId, moves: &[(i64, i64)]) -> Case {
    let (seq, ids) = (fx.seq, clip_ids(&fx.project, fx.seq));
    let end = clips.iter().map(|c| c.0 + c.1).max().unwrap_or(0);
    let k = rng.int(0, clips.len() as i64 - 1) as usize;
    let (c, l, _) = clips[k];
    let mut after = fx.project.clone();
    match rng.int(0, 6) {
        0 => {
            let (x, y) = cut_range(rng, moves, end);
            ripple_delete(&mut after, seq, x, y);
            Case { after, map: deleted(end, x, y), cuts: inside(vec![x], end - (y - x)), what: format!("ripple delete [{x}, {y})") }
        }
        1 => {
            let x = inside_a_move(rng, moves).unwrap_or_else(|| rng.int(0, end));
            let n = rng.int(1, 40);
            ripple_insert(&mut after, seq, x, other, n, rng.int(0, 1_000));
            Case { after, map: inserted(end, x, n), cuts: inside(vec![x, x + n], end + n), what: format!("ripple insert of {n} at {x}") }
        }
        2 => {
            let by = if rng.chance(0.5) { -rng.int(1, l - 1) } else { rng.int(1, 30) };
            trim_tail(&mut after, seq, ids[k], by);
            let tail = c + l;
            let map = (0..end)
                .map(|f| {
                    if f < tail + by.min(0) {
                        Some(f)
                    } else if f < tail {
                        None
                    } else {
                        Some(f + by)
                    }
                })
                .collect();
            Case { after, map, cuts: inside(vec![tail + by], end + by), what: format!("tail trim of clip {k} by {by}") }
        }
        3 => {
            let by = rng.int(1, l - 1);
            trim_head(&mut after, seq, ids[k], by);
            let map = (0..end)
                .map(|f| {
                    if f < c {
                        Some(f)
                    } else if f < c + by {
                        None
                    } else {
                        Some(f - by)
                    }
                })
                .collect();
            Case { after, map, cuts: inside(vec![c], end - by), what: format!("head trim of clip {k} by {by}") }
        }
        4 if k + 1 < clips.len() => {
            let l2 = clips[k + 1].1;
            swap(&mut after, seq, k);
            let map = (0..end)
                .map(|f| {
                    Some(if f < c || f >= c + l + l2 {
                        f
                    } else if f < c + l {
                        f + l2
                    } else {
                        f - l
                    })
                })
                .collect();
            Case { after, map, cuts: inside(vec![c, c + l2, c + l + l2], end), what: format!("swap of clips {k} and {}", k + 1) }
        }
        5 => {
            replace(&mut after, seq, k, other, 2_000);
            let map = (0..end).map(|f| (f < c || f >= c + l).then_some(f)).collect();
            Case { after, map, cuts: inside(vec![c, c + l], end), what: format!("replace of clip {k}") }
        }
        _ => {
            let (x, y) = cut_range(rng, moves, end);
            ripple_delete(&mut after, seq, x, y);
            let shorter = end - (y - x);
            let (z, n) = (rng.int(0, shorter), rng.int(1, 40));
            ripple_insert(&mut after, seq, z, other, n, rng.int(0, 1_000));
            let junction = if x < z { x } else { x + n };
            let (map, cuts) = (then(&deleted(end, x, y), &inserted(shorter, z, n)), inside(vec![junction, z, z + n], shorter + n));
            Case { after, map, cuts, what: format!("ripple delete [{x}, {y}) then insert of {n} at {z}") }
        }
    }
}

#[test]
fn t_m11_random_edits_keep_every_surviving_window_within_two_split_bounds() {
    let mut rng = Rng::new(0x7e57_0011);
    for case in 0..300 {
        let clips = random_clips(&mut rng);
        let mut fx = Fx::with_clips(&clips);
        let other = add_matte(&mut fx.project, "insert");
        let r = fx.r();
        fx.install(one_plan(random_positions(&mut rng, &r, &clips)));
        assert!(validate(&fx.project, fx.seq()).is_empty(), "case {case}: the generated plan is valid");
        let old = plan_in(&fx.project, fx.seq, 1).clone();
        let moves: Vec<(i64, i64)> = (1..old.positions.len()).filter_map(|k| move_span(&old, &r, k)).map(|m| (m.start_frame, m.end_frame)).collect();
        let Case { mut after, map, cuts, what } = random_edit(&mut rng, &fx, &clips, other, &moves);
        let what = format!("case {case}, {what}");
        let report = follow(&fx.project, &mut after);
        assert!(validate(&after, seq_of(&after, fx.seq)).is_empty(), "{what}: the followed plan is valid");
        assert_follows(&fx.project, &after, fx.seq, 1, &map, &what);
        let new = plan_in(&after, fx.seq, 1);
        let (mut moved, mut removed, mut locked) = (Vec::new(), Vec::new(), Vec::new());
        for p in &old.positions {
            let f = fr(p.at);
            let Some(g) = map.get(f as usize).copied().flatten() else {
                assert!(find(new, p.id).is_none(), "{what}: #{} on removed frame {f} goes", p.id);
                removed.push(p.id);
                if p.locked {
                    locked.push(p.id);
                }
                continue;
            };
            let kept = find(new, p.id).unwrap_or_else(|| panic!("{what}: #{} on surviving frame {f} is kept", p.id));
            assert_eq!(fr(kept.at), g, "{what}: #{} follows its picture", p.id);
            assert!(same_bits(&kept.window, &p.window), "{what}: #{} keeps its window", p.id);
            assert_eq!((kept.layout, kept.reviewed, kept.locked, &kept.label), (p.layout, p.reviewed, p.locked, &p.label), "{what}: #{}", p.id);
            assert!(kept.flags.iter().all(|flag| *flag == Flag::PictureChanged), "{what}: #{} was remapped exactly: {:?}", p.id, kept.flags);
            if g != f {
                moved.push(p.id);
            }
        }
        for added in new.positions.iter().filter(|p| find(&old, p.id).is_none()) {
            assert!(added.id >= old.next_position_id, "{what}: an added position takes a new id");
            assert_eq!((added.origin, added.reviewed, added.locked), (Origin::Reanchored, false, false), "{what}: added #{}", added.id);
        }
        let walls_now = wall_frames_of(&after, fx.seq);
        for c in &cuts {
            assert!(walls_now.contains(c), "{what}: the junction at {c} is a wall");
        }
        assert!(stored_walls(&after, fx.seq, WallOrigin::Reanchored).is_empty(), "{what}: top-level junctions are edit walls");
        moved.sort_unstable();
        removed.sort_unstable();
        locked.sort_unstable();
        let pr = plan_report(&report, fx.seq, 1);
        assert_eq!((pr.moved, pr.removed, pr.locked), (moved, removed, locked), "{what}: the report");
    }
}

#[test]
fn t_m11_an_opener_follows_its_clips_extended_head_and_is_flagged() {
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 100, 10_000)]);
    let opener = Position { label: "B".into(), reviewed: true, ..at25(2, 100, full(1200.0), Arrival::Cut) };
    fx.install(one_plan(vec![at25(1, 0, full(600.0), Arrival::Cut), opener, at25(3, 150, full(900.0), mv(30, Curve::Linear))]));
    let ids = clip_ids(&fx.project, fx.seq);
    let before = fx.project.clone();
    // a roll: B's head takes 10 frames of A's tail and shows 10 more frames of its own media
    let mut after = edited(&before, |p| roll(p, fx.seq, ids[0], ids[1], 10));
    let report = follow(&before, &mut after);
    let new = plan_in(&after, fx.seq, 1);
    assert_eq!(frames_of(new), vec![0, 90, 150]);
    let moved = find(new, 2).expect("the opener survives");
    assert_eq!(fr(moved.at), 90, "the opener sits on B's new first frame");
    assert!(same_bits(&moved.window, &full(1200.0)));
    assert_eq!(moved.flags, vec![Flag::PictureChanged], "the new head frames take its window: flagged");
    assert!(!moved.reviewed, "moving it onto the extended head is no exact remap");
    assert_eq!(moved.label, "B");
    let map: Vec<Option<i64>> = (0..200).map(|f| (!(90..100).contains(&f)).then_some(f)).collect();
    assert_follows(&before, &after, fx.seq, 1, &map, "a roll");
    let r1 = raster(seq_of(&after, fx.seq));
    for g in 90..100 {
        assert!(same_bits(&window_at(new, &r1, tk(g)).window, &full(1200.0)), "new head frame {g} shows the opener's window");
    }
    let pr = plan_report(&report, fx.seq, 1);
    assert_eq!((pr.moved, pr.flagged, pr.added, pr.removed), (vec![2], vec![2], vec![], vec![]));
    assert!(validate(&after, seq_of(&after, fx.seq)).is_empty());
}

#[test]
fn t_m11_a_trimmed_head_gets_an_exact_opener_and_the_old_one_goes() {
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 100, 10_000)]);
    let opener = Position { label: "B".into(), reviewed: true, ..at25(2, 100, full(1200.0), Arrival::Cut) };
    fx.install(one_plan(vec![at25(1, 0, full(600.0), Arrival::Cut), opener, at25(3, 150, full(900.0), mv(30, Curve::Linear))]));
    let b = clip_ids(&fx.project, fx.seq)[1];
    let before = fx.project.clone();
    let mut after = edited(&before, |p| trim_head(p, fx.seq, b, 10));
    let report = follow(&before, &mut after);
    let new = plan_in(&after, fx.seq, 1);
    assert_eq!(frames_of(new), vec![0, 100, 140]);
    assert_eq!(ids_of(new), vec![1, 4, 3]);
    assert!(find(new, 2).is_none(), "the old opener's frame is gone, and the opener with it");
    let opener = find(new, 4).expect("an exact opener on B's new first frame");
    assert!(same_bits(&opener.window, &full(1200.0)));
    assert_eq!((opener.arrival, opener.origin, opener.reviewed, opener.label.as_str()), (Arrival::Cut, Origin::Reanchored, false, ""));
    assert_eq!(find(new, 3).expect("#3").arrival, mv(30, Curve::Linear), "the move maps contiguously and keeps its length");
    assert_follows(&before, &after, fx.seq, 1, &deleted(200, 100, 110), "a head trim");
    let pr = plan_report(&report, fx.seq, 1);
    assert_eq!((pr.moved, pr.removed, pr.added), (vec![3], vec![2], vec![4]));
    assert!(validate(&after, seq_of(&after, fx.seq)).is_empty());
}

#[test]
fn t_m11_new_content_carries_the_window_before_it_and_resumed_content_gets_an_opener() {
    let mut fx = Fx::with_clips(&[(0, 200, 0)]);
    let other = add_matte(&mut fx.project, "insert");
    let held = full(1000.0);
    fx.install(one_plan(vec![at25(1, 0, full(600.0), Arrival::Cut), at25(2, 50, held, Arrival::Cut), at25(3, 150, full(1300.0), mv(40, Curve::EaseInOut))]));
    let before = fx.project.clone();
    let mut after = edited(&before, |p| {
        ripple_insert(p, fx.seq, 80, other, 30, 0);
    });
    let report = follow(&before, &mut after);
    let new = plan_in(&after, fx.seq, 1);
    assert_eq!(frames_of(new), vec![0, 50, 110, 180]);
    assert_eq!(ids_of(new), vec![1, 2, 4, 3]);
    let opener = find(new, 4).expect("the resumed content opens on its own position");
    assert!(same_bits(&opener.window, &held));
    assert_eq!((opener.arrival, opener.origin, opener.reviewed), (Arrival::Cut, Origin::Reanchored, false));
    assert_eq!(find(new, 2).expect("#2").flags, vec![Flag::PictureChanged], "#2 now also frames the inserted clip");
    assert_eq!(find(new, 3).expect("#3").arrival, mv(40, Curve::EaseInOut));
    let r1 = raster(seq_of(&after, fx.seq));
    for g in 80..110 {
        assert!(same_bits(&window_at(new, &r1, tk(g)).window, &held), "inserted frame {g} carries #2's window");
    }
    let list = shots(&after, seq_of(&after, fx.seq), 1);
    let inserted_shot = list.iter().find(|s| s.from == 80).expect("the inserted clip is a shot");
    assert_eq!((inserted_shot.status, inserted_shot.carried_from), (ShotStatus::Carried, Some(2)));
    let resumed = list.iter().find(|s| s.from == 110).expect("the resumed content is a shot");
    assert_eq!((resumed.status, resumed.opener), (ShotStatus::Own, Some(4)));
    assert_follows(&before, &after, fx.seq, 1, &inserted(200, 80, 30), "an insert");
    let pr = plan_report(&report, fx.seq, 1);
    assert_eq!((pr.moved, pr.flagged, pr.added, pr.removed), (vec![3], vec![2], vec![4], vec![]));
}

#[test]
fn t_m11_suppressions_follow_their_picture_and_one_on_removed_content_is_dropped() {
    let mut fx = Fx::with_clips(&[(0, 300, 0)]);
    let other = add_matte(&mut fx.project, "insert");
    let mut set = one_plan(vec![at25(1, 0, full(600.0), Arrival::Cut)]);
    let detected = StoredWall { at: tk(100), origin: WallOrigin::Detected, score: Some(0.5) };
    set.walls.stored = vec![detected];
    set.walls.suppressed = vec![tk(100), tk(200), tk(210), tk(240)];
    set.walls.candidates = vec![Candidate { at: tk(250), score: 0.25 }];
    fx.install(set);
    let before = fx.project.clone();
    let suppressed = |p: &Project| set_in(p, fx.seq).walls.suppressed.iter().map(|t| fr(*t)).collect::<Vec<i64>>();
    // an insert at 150: the suppression before it stays on its picture, the ones after it follow theirs
    let mut after = edited(&before, |p| {
        ripple_insert(p, fx.seq, 150, other, 20, 0);
    });
    let report = follow(&before, &mut after);
    assert_eq!(suppressed(&after), vec![100, 220, 230, 260]);
    assert_eq!(set_in(&after, fx.seq).walls.stored, vec![detected], "the deleted detected wall stays, still suppressed");
    assert_eq!(set_in(&after, fx.seq).walls.candidates, vec![Candidate { at: tk(270), score: 0.25 }]);
    assert!(report_changes(&report, fx.seq, "suppression_dropped").is_empty());
    // a ripple delete of [190, 210): the suppression on removed content goes, and so does the one that would land on the
    // junction (a junction is always a wall); both are reported; the one at 240 follows its picture
    let mut after = edited(&before, |p| ripple_delete(p, fx.seq, 190, 210));
    let report = follow(&before, &mut after);
    assert_eq!(suppressed(&after), vec![100, 220]);
    let mut dropped: Vec<serde_json::Value> = report_changes(&report, fx.seq, "suppression_dropped").iter().map(|c| c.detail["frame"].clone()).collect();
    dropped.sort_by_key(|v| v.as_i64());
    assert_eq!(dropped, vec![json!(200), json!(210)]);
    assert_eq!(set_in(&after, fx.seq).walls.candidates, vec![Candidate { at: tk(230), score: 0.25 }]);
    assert!(wall_frames_of(&after, fx.seq).contains(&190), "the junction is a wall");
}

#[test]
fn t_m11_a_ripple_inside_a_nest_makes_a_reanchored_wall_in_the_parent_and_no_move_crosses_it() {
    let mut fx = Fx::with_clips(&[]);
    let (nest, _) = add_nest(&mut fx, 400, 0);
    let item = fx.make_item(nest, tk(0), 300, 0);
    fx.put(0, item);
    fx.install(one_plan(vec![
        at25(1, 0, full(600.0), Arrival::Cut),
        at25(2, 100, full(1000.0), Arrival::Cut),
        at25(3, 160, full(1400.0), mv(40, Curve::Linear)),
    ]));
    let before = fx.project.clone();
    // inside the nest: frames 130-139 go; the parent's nest clip keeps its length and shows the nest's frames 140-309 at 130
    let mut after = edited(&before, |p| ripple_delete(p, nest, 130, 140));
    let report = follow(&before, &mut after);
    let (r, old, new) = (fx.r(), plan_in(&before, fx.seq, 1), plan_in(&after, fx.seq, 1));
    assert_eq!(frames_of(new), vec![0, 100, 129, 130, 150]);
    assert_eq!(ids_of(new), vec![1, 2, 4, 5, 3]);
    let (left, right) = (find(new, 4).expect("#4"), find(new, 5).expect("#5"));
    assert!(same_bits(&left.window, &split_window(old, &r, 129)));
    assert_eq!(left.arrival, mv_split(9, Curve::Linear, 0, 40));
    assert!(same_bits(&right.window, &split_window(old, &r, 140)));
    assert_eq!(right.arrival, Arrival::Cut, "no move crosses the junction");
    assert_eq!(find(new, 3).expect("#3").arrival, mv_split(20, Curve::Linear, 20, 40));
    assert_eq!(stored_walls(&after, fx.seq, WallOrigin::Reanchored), vec![130], "the nest's ripple left no edit point in the parent");
    let wall = walls(&after, seq_of(&after, fx.seq)).into_iter().find(|w| fr(w.at) == 130).expect("a wall at the junction");
    assert_eq!(wall.sources, vec![WallSource::Reanchored]);
    assert!(!wall_frames_of(&after, fx.seq).contains(&290), "the nest's picture continues at 290: no wall there");
    assert_follows(&before, &after, fx.seq, 1, &deleted(300, 130, 140), "a ripple inside a nest");
    assert!(validate(&after, seq_of(&after, fx.seq)).is_empty());
    let pr = plan_report(&report, fx.seq, 1);
    assert_eq!((pr.moved, pr.added, pr.removed), (vec![3], vec![4, 5], vec![]));
}

#[test]
fn t_m11_moved_clips_carry_their_framing_exactly() {
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 150, 10_000), (250, 100, 20_000)]);
    fx.install(one_plan(vec![
        at25(1, 10, full(600.0), Arrival::Cut),
        at25(2, 40, full(700.0), mv(20, Curve::Linear)),
        at25(3, 100, full(1200.0), Arrival::Cut),
        at25(4, 200, win(1000.0, 600.0, 900.0), mv(60, Curve::EaseInOut)),
        at25(5, 250, full(800.0), Arrival::Cut),
    ]));
    let before = fx.project.clone();
    // an insert move of A after B: B [0, 150), A [150, 250), C stays
    let mut after = edited(&before, |p| swap(p, fx.seq, 0));
    let report = follow(&before, &mut after);
    let new = plan_in(&after, fx.seq, 1);
    assert_eq!(frames_of(new), vec![0, 100, 150, 160, 190, 250]);
    assert_eq!(ids_of(new), vec![3, 4, 6, 1, 2, 5]);
    let anchor = find(new, 6).expect("A's first frames showed #1's window before #1: an exact anchor keeps them");
    assert!(same_bits(&anchor.window, &full(600.0)));
    assert_eq!((anchor.arrival, anchor.origin), (Arrival::Cut, Origin::Reanchored));
    assert_eq!(find(new, 4).expect("#4").arrival, mv(60, Curve::EaseInOut));
    assert_eq!(find(new, 2).expect("#2").arrival, mv(20, Curve::Linear));
    let map: Vec<Option<i64>> = (0..350)
        .map(|f| {
            Some(if f < 100 {
                f + 150
            } else if f < 250 {
                f - 100
            } else {
                f
            })
        })
        .collect();
    assert_follows(&before, &after, fx.seq, 1, &map, "a swap");
    let pr = plan_report(&report, fx.seq, 1);
    assert_eq!((pr.moved, pr.removed, pr.added, pr.flagged), (vec![1, 2, 3, 4], vec![], vec![6], vec![]));
    assert!(stored_walls(&after, fx.seq, WallOrigin::Reanchored).is_empty());
}

#[test]
fn t_m11_a_replaced_clip_takes_its_positions_away_and_its_shot_reads_carried() {
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 100, 10_000), (200, 100, 20_000)]);
    let other = add_matte(&mut fx.project, "replacement");
    fx.install(one_plan(vec![
        at25(1, 0, full(600.0), Arrival::Cut),
        at25(2, 100, full(1200.0), Arrival::Cut),
        at25(3, 150, full(900.0), mv(30, Curve::Linear)),
        at25(4, 200, full(800.0), Arrival::Cut),
    ]));
    let before = fx.project.clone();
    let mut after = edited(&before, |p| replace(p, fx.seq, 1, other, 2_000));
    let report = follow(&before, &mut after);
    let new = plan_in(&after, fx.seq, 1);
    assert_eq!((ids_of(new), frames_of(new)), (vec![1, 4], vec![0, 200]));
    assert_eq!(find(new, 1).expect("#1").flags, vec![Flag::PictureChanged], "#1's window now carries over the new clip");
    let list = shots(&after, seq_of(&after, fx.seq), 1);
    let replaced = list.iter().find(|s| s.from == 100).expect("the new clip is a shot");
    assert_eq!((replaced.status, replaced.carried_from), (ShotStatus::Carried, Some(1)));
    let map: Vec<Option<i64>> = (0..300).map(|f| (!(100..200).contains(&f)).then_some(f)).collect();
    assert_follows(&before, &after, fx.seq, 1, &map, "a replaced clip");
    let pr = plan_report(&report, fx.seq, 1);
    assert_eq!((pr.removed, pr.flagged, pr.added, pr.moved), (vec![2, 3], vec![1], vec![], vec![]));
}

#[test]
fn t_m11_a_retimed_clip_maps_to_the_nearest_frames_and_flags_time_approximate() {
    let mut fx = Fx::with_clips(&[(0, 300, 0), (300, 100, 10_000)]);
    fx.install(one_plan(vec![
        at25(1, 0, full(600.0), Arrival::Cut),
        Position { reviewed: true, ..at25(2, 31, full(700.0), Arrival::Cut) },
        at25(3, 91, full(900.0), mv(30, Curve::Linear)),
        Position { locked: true, ..at25(4, 150, full(800.0), Arrival::Cut) },
        Position { locked: true, ..at25(6, 200, full(1100.0), Arrival::Cut) },
        at25(5, 300, full(1200.0), Arrival::Cut),
    ]));
    let a = clip_ids(&fx.project, fx.seq)[0];
    let before = fx.project.clone();
    // A at 150 %: frame g shows media frame 1.5 g, so old frame f lands on the frame nearest 2 f / 3
    let mut after = edited(&before, |p| retime(p, fx.seq, a, 1.5, 200));
    let report = follow(&before, &mut after);
    let new = plan_in(&after, fx.seq, 1);
    assert_eq!(ids_of(new), vec![1, 2, 3, 4, 6, 5]);
    assert_eq!(frames_of(new), vec![0, 21, 61, 100, 133, 200]);
    let flags = |id: u32| find(new, id).expect("kept").flags.clone();
    // the clip's picture now plays faster under every position that frames it ("timing relative to the span"); a position
    // that lands between media frames is also approximate; B moved exactly
    for id in [1, 4] {
        assert_eq!(flags(id), vec![Flag::PictureChanged], "#{id} maps exactly into the retimed clip");
    }
    for id in [2, 3, 6] {
        let f = flags(id);
        assert!(f.len() == 2 && f.contains(&Flag::TimeApproximate) && f.contains(&Flag::PictureChanged), "#{id} lands between media frames: {f:?}");
    }
    assert!(flags(5).is_empty(), "B's opener moved exactly");
    assert!(!find(new, 2).expect("#2").reviewed, "an inexact remap does not keep the review");
    assert_eq!(find(new, 3).expect("#3").arrival, mv(20, Curve::Linear), "the move runs from the frame nearest its start");
    assert!(find(new, 4).expect("#4").locked && find(new, 6).expect("#6").locked);
    let pr = plan_report(&report, fx.seq, 1);
    assert_eq!((pr.moved, pr.flagged, pr.locked, pr.removed), (vec![2, 3, 4, 5, 6], vec![1, 2, 3, 4, 6], vec![6], vec![]));
    assert!(validate(&after, seq_of(&after, fx.seq)).is_empty());
}

#[test]
fn t_m11_two_positions_landing_on_one_frame_keep_the_earlier() {
    let mut fx = Fx::with_clips(&[(0, 200, 0)]);
    fx.install(one_plan(vec![at25(1, 0, full(600.0), Arrival::Cut), at25(2, 50, full(800.0), Arrival::Cut), at25(3, 51, full(1000.0), Arrival::Cut)]));
    let a = clip_ids(&fx.project, fx.seq)[0];
    let before = fx.project.clone();
    // at 200 % media frame 51 lies between frames 25 and 26 (media 50 and 52): a tie goes to the earlier frame, 25, which
    // #2 holds already
    let mut after = edited(&before, |p| retime(p, fx.seq, a, 2.0, 100));
    let report = follow(&before, &mut after);
    let new = plan_in(&after, fx.seq, 1);
    assert_eq!((ids_of(new), frames_of(new)), (vec![1, 2], vec![0, 25]));
    assert_eq!(plan_report(&report, fx.seq, 1).removed, vec![3]);
}

#[test]
fn t_m11_a_frame_held_clip_maps_by_offset() {
    let mut fx = Fx::with_clips(&[(0, 200, 0)]);
    let a = clip_ids(&fx.project, fx.seq)[0];
    fx.item_mut(a).frame_hold = Some(tk(50));
    fx.install(one_plan(vec![
        at25(1, 0, full(600.0), Arrival::Cut),
        at25(2, 60, full(1000.0), Arrival::Cut),
        at25(3, 120, full(1300.0), mv(30, Curve::Linear)),
    ]));
    let before = fx.project.clone();
    let mut after = edited(&before, |p| ripple_delete(p, fx.seq, 20, 40));
    let report = follow(&before, &mut after);
    let new = plan_in(&after, fx.seq, 1);
    assert_eq!((ids_of(new), frames_of(new)), (vec![1, 2, 3], vec![0, 40, 100]));
    assert_eq!(find(new, 3).expect("#3").arrival, mv(30, Curve::Linear));
    assert!(new.positions.iter().all(|p| p.flags.is_empty()));
    assert_follows(&before, &after, fx.seq, 1, &deleted(200, 20, 40), "a ripple inside a frame hold");
    assert!(stored_walls(&after, fx.seq, WallOrigin::Reanchored).is_empty(), "both pieces show the held frame: no cut, no wall");
    let pr = plan_report(&report, fx.seq, 1);
    assert_eq!((pr.moved, pr.added, pr.removed), (vec![2, 3], vec![], vec![]));
}

#[test]
fn t_m11_a_persons_edit_removes_a_locked_position_on_removed_content_and_reports_it() {
    let mut fx = Fx::with_clips(&[(0, 200, 0), (200, 100, 10_000)]);
    let lock = |p: Position| Position { locked: true, ..p };
    fx.install(one_plan(vec![
        at25(1, 0, full(600.0), Arrival::Cut),
        lock(at25(2, 50, full(900.0), Arrival::Cut)),
        lock(at25(3, 100, win(1300.0, 600.0, 900.0), mv(40, Curve::EaseInOut))),
        lock(at25(4, 150, full(1000.0), Arrival::Cut)),
        at25(5, 200, full(800.0), Arrival::Cut),
    ]));
    let before = fx.project.clone();
    let mut after = edited(&before, |p| ripple_delete(p, fx.seq, 40, 70));
    let report = follow(&before, &mut after);
    let new = plan_in(&after, fx.seq, 1);
    assert_eq!((ids_of(new), frames_of(new)), (vec![1, 6, 3, 4, 5], vec![0, 40, 70, 120, 170]));
    let split = find(new, 3).expect("#3");
    assert!(split.locked, "a locked position the hook keeps exactly stays locked");
    assert_eq!(split.arrival, mv_split(30, Curve::EaseInOut, 10, 40), "its move is split exactly at the junction");
    assert!(find(new, 4).expect("#4").locked);
    let pr = plan_report(&report, fx.seq, 1);
    assert_eq!((pr.removed, pr.locked), (vec![2], vec![2]), "for a person the locked position goes, and the report says so");
    assert_follows(&before, &after, fx.seq, 1, &deleted(300, 40, 70), "a ripple delete over locked positions");
}

#[test]
fn t_m11_a_move_that_now_holds_a_wall_is_shortened_and_flagged() {
    let mut fx = Fx::with_clips(&[(0, 300, 0)]);
    fx.install(one_plan(vec![at25(1, 0, full(600.0), Arrival::Cut), Position { locked: true, ..at25(2, 100, full(1300.0), mv(60, Curve::Linear)) }]));
    let before = fx.project.clone();
    // Add Edit at 70, then a punch-in on the right piece: a through edit whose pieces differ in Motion is a wall
    let mut after = edited(&before, |p| {
        let right = razor(p, fx.seq, 70);
        set_param(clip_mut(p, fx.seq, right), "motion", "scale", ParamValue::Float(120.0));
    });
    let report = follow(&before, &mut after);
    assert!(wall_frames_of(&after, fx.seq).contains(&70), "a punch-in is a shot change");
    let moved = find(plan_in(&after, fx.seq, 1), 2).expect("#2");
    assert_eq!(moved.arrival, mv(30, Curve::Linear), "the move now starts at the wall");
    assert!(moved.flags.contains(&Flag::MoveShortened) && moved.flags.contains(&Flag::PictureChanged), "{:?}", moved.flags);
    assert!(moved.locked);
    let pr = plan_report(&report, fx.seq, 1);
    assert_eq!((pr.flagged, pr.locked, pr.moved), (vec![1, 2], vec![2], vec![]), "frame 70 shows #1's window over the new picture");
    assert!(validate(&after, seq_of(&after, fx.seq)).is_empty());
}

#[test]
fn t_m11_the_hook_writes_only_the_after_projects_vertical_data() {
    let mut fx = Fx::with_clips(&[(0, 400, 0)]);
    fx.install(one_plan(vec![at25(1, 100, full(812.4), Arrival::Cut), at25(2, 150, full(960.0), mv(50, Curve::EaseInOut))]));
    let before = fx.project.clone();
    let edit = edited(&before, |p| ripple_delete(p, fx.seq, 120, 130));
    let mut after = edit.clone();
    follow(&before, &mut after);
    assert!(same_project(&before, &fx.project), "the before project is untouched: undo restores it bit for bit");
    assert_ne!(set_in(&after, fx.seq), set_in(&before, fx.seq), "the hook wrote the after project's plan");
    let strip = |p: &Project| {
        edited(p, |q| {
            for item in q.items.values_mut() {
                if let Some(s) = item.as_sequence_mut() {
                    s.vertical = None;
                }
            }
        })
    };
    assert!(same_project(&strip(&after), &strip(&edit)), "the hook changes nothing but vertical data");
}

#[test]
fn t_m11_the_report_counts_positions_and_gives_the_undo_label_suffix() {
    let plan = PlanFollow { plan: 1, moved: (1..=12).collect(), removed: vec![13], flagged: vec![2, 3], added: Vec::new(), locked: Vec::new() };
    let one = FollowReport { sequences: vec![SequenceFollow { sequence: ItemId(7), plans: vec![plan], changes: Vec::new(), issues: Vec::new() }] };
    assert_eq!((one.moved(), one.removed(), one.flagged(), one.added()), (12, 1, 2, 0));
    assert_eq!(one.label_suffix().as_deref(), Some("· vertical: 12 moved, 1 removed, 2 flagged"));
    assert!(!one.is_empty());
    let mut sequences = one.sequences.clone();
    let other = PlanFollow { plan: 2, moved: vec![4], added: vec![9, 10], ..PlanFollow::default() };
    sequences.push(SequenceFollow { sequence: ItemId(8), plans: vec![other], ..SequenceFollow::default() });
    let two = FollowReport { sequences };
    assert_eq!((two.moved(), two.removed(), two.flagged(), two.added()), (13, 1, 2, 2));
    assert_eq!(two.label_suffix().as_deref(), Some("· vertical: 13 moved, 1 removed, 2 flagged, 2 added"));
    let empty = FollowReport::default();
    assert!(empty.is_empty());
    assert_eq!(empty.label_suffix(), None);
    let walls_only = Change { code: "wall_stored", detail: json!({"frame": 130}) };
    let data = FollowReport { sequences: vec![SequenceFollow { sequence: ItemId(7), changes: vec![walls_only], ..SequenceFollow::default() }] };
    assert!(!data.is_empty(), "the hook changed the set's data");
    assert_eq!(data.label_suffix(), None, "no position changed");
}

#[test]
fn t_m11_the_bound_is_two_split_bounds_inside_moves_and_zero_where_the_window_holds() {
    let r = hd();
    let plan = mixed_plan(&r);
    let mut inside_moves = 0;
    for f in 0..800 {
        let (expected, got) = (bound_at(&plan, &r, f), follow_bound(&plan, &r, f));
        assert!((got - expected).abs() <= 1e-15, "frame {f}: {got} against {expected}");
        if expected > 0.0 {
            inside_moves += 1;
        }
    }
    assert!(inside_moves > 300, "the plan's moves are checked: {inside_moves} frames");
    let pan = hook_bound(&full(600.0), &full(900.0), OUT);
    assert!((pan - 0.0025).abs() < 1e-15, "a pure pan: two clamped deltas, {pan}");
    let zoom = hook_bound(&full(600.0), &win(900.0, 600.0, 540.0), OUT);
    assert!((zoom - two_splits(&full(600.0), &win(900.0, 600.0, 540.0), OUT)).abs() < 1e-15, "{zoom}");
    assert!(zoom > pan, "a zoom adds the travel term");
}
