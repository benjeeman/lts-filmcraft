//! T-M3: random valid plans (every curve, zooms, `fit` positions and split moves) keep every evaluated window inside the
//! frame within 1e-6 px and inside the height range (spec sections 3.5 and 9, C6).

use filmcraft_time::Tick;

use super::support::*;
use crate::vertical::*;

/// A random valid plan: positions in frame order, windows on the grid inside the frame, moves that fit their gaps, `fit`
/// positions reached and left by cuts, and up to three exact splits.
fn random_plan(rng: &mut Rng, r: &Raster) -> VerticalPlan {
    let mut positions = Vec::new();
    let mut frame = rng.int(0, 40);
    let mut previous_fit = false;
    for id in 1..=rng.int(1, 8) {
        let gap = rng.int(1, 90);
        if id > 1 {
            frame += gap;
        }
        let is_fit = rng.chance(0.15);
        let window = random_window(rng, r, OUT);
        let cut = id == 1 || is_fit || previous_fit || gap < 2 || rng.chance(0.2);
        let arrival = if cut { Arrival::Cut } else { mv(rng.int(2, gap) as u32, random_curve(rng)) };
        let layout = if is_fit { Layout::Fit } else { Layout::Fill };
        positions.push(Position { layout, ..pos(r, id as u32, frame, window, arrival) });
        previous_fit = is_fit;
    }
    let mut plan = plan_of(positions);
    let first = plan.positions.first().map_or(0, |p| r.rate.frame_at(p.at));
    let last = plan.positions.last().map_or(0, |p| r.rate.frame_at(p.at));
    for _ in 0..3 {
        if let Some(split) = split_move(&plan, r, rng.int(first, last)) {
            let id = plan.next_position_id;
            insert_split(&mut plan, &split, id, Origin::Manual, Writer::Human);
            plan.next_position_id += 1;
        }
    }
    plan
}

#[test]
fn t_m3_random_valid_plans_stay_inside_the_frame() {
    let mut rng = Rng::new(0x7e57_0003);
    for (r, cases) in [(hd(), 150), (uhd(), 60)] {
        let half = r.rate.frame_duration().0 / 2;
        for case in 0..cases {
            let plan = random_plan(&mut rng, &r);
            let first = plan.positions.first().map_or(0, |p| r.rate.frame_at(p.at));
            let last = plan.positions.last().map_or(0, |p| r.rate.frame_at(p.at));
            for f in first - 3..=last + 3 {
                for t in [tick(&r, f), Tick(tick(&r, f).0 + half)] {
                    let w = window_at(&plan, &r, t).window;
                    assert!(inside(&w, &r, OUT, 1e-6), "case {case}, tick {}: {w:?} leaves the frame or the height range", t.0);
                }
            }
        }
    }
}

#[test]
fn t_m3_the_mixed_plan_stays_inside_on_both_rasters() {
    for r in [hd(), uhd()] {
        let plan = mixed_plan(&r);
        let last = plan.positions.last().map_or(0, |p| r.rate.frame_at(p.at));
        for f in -2..=last + 2 {
            let w = window_at(&plan, &r, tick(&r, f)).window;
            assert!(inside(&w, &r, OUT, 1e-6), "frame {f}: {w:?}");
        }
    }
}
