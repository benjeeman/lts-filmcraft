//! T-M31 (spec section 4.11): the active picture area constrains the helpers (their clamp, Alt+0's largest window,
//! `vertical.fit`, `seedShots {mode: "starting"}`), raises `window_outside_active` where the window shows bars, and never
//! changes stored data or the invariants (I3 stays the frame). And the writes of `vertical.setActiveArea`.

use serde_json::json;

use super::fixtures::*;
use super::support::win;
use crate::vertical::*;

/// A 2.39:1 letterbox inside 1920x1080: picture in rows 132 to 948.
const LETTERBOX: [f64; 4] = [0.0, 132.0, 1920.0, 816.0];

fn area(from: i64, to: i64, rect: [f64; 4]) -> ActiveArea {
    ActiveArea { from: tk(from), to: tk(to), rect, origin: Origin::Manual, modified_by: Writer::Human }
}

/// A full-height window until 300, then a window inside the letterbox; the letterbox over [0, 500).
fn letterboxed() -> Fx {
    let mut fx = Fx::new(1_000);
    let mut set = one_plan(vec![at25(1, 0, win(960.0, 540.0, 1080.0), Arrival::Cut), at25(2, 300, win(960.0, 540.0, 816.0), Arrival::Cut)]);
    set.active_area = vec![area(0, 500, LETTERBOX)];
    fx.install(set);
    fx
}

#[test]
fn t_m31_the_active_rect_is_the_area_over_its_range_and_the_frame_elsewhere() {
    let fx = letterboxed();
    let r = fx.r();
    assert_eq!(active_rect(fx.set(), &r, tk(100)), LETTERBOX);
    assert_eq!(active_rect(fx.set(), &r, tk(499)), LETTERBOX);
    assert_eq!(active_rect(fx.set(), &r, tk(500)), [0.0, 0.0, 1920.0, 1080.0]);
}

#[test]
fn t_m31_the_helpers_clamp_stays_inside_the_area() {
    let fx = letterboxed();
    let r = fx.r();
    assert_eq!(largest_in_area(LETTERBOX, &r, OutputSize { width: 1080, height: 1920 }), win(960.0, 540.0, 816.0), "Alt+0 in the letterbox");
    let out = fx.plan(1).output;
    assert_eq!(clamp_to_area(win(960.0, 540.0, 1080.0), LETTERBOX, &r, out), win(960.0, 540.0, 816.0), "h first, at most the area's height");
    assert_eq!(clamp_to_area(win(100.0, 300.0, 600.0), LETTERBOX, &r, out), win(168.75, 432.0, 600.0), "then the centre, inward");
    let small = [800.0, 400.0, 100.0, 100.0];
    let tiny = clamp_to_area(win(900.0, 500.0, 900.0), small, &r, out);
    assert_eq!(tiny.h, 480.0, "never below the smallest window");
    assert_eq!((tiny.cx, tiny.cy), (850.0, 450.0), "an area too small for it: its centre");
}

#[test]
fn t_m31_vertical_fit_stays_inside_the_area() {
    let fx = letterboxed();
    let req = FitRequest { frame: 100, regions: vec![[100.0, 140.0, 200.0, 200.0]], ..Default::default() };
    let result = fit::fit(&fx.project, fx.seq(), 1, &req).expect("fitted");
    assert_eq!(result.window, win(200.0, 372.0, 480.0), "the smallest window, its centre moved down into the letterbox");
    assert!(result.fits);
    assert_eq!(result.visible, vec![1.0]);
    assert_eq!(result.limited_by, Some(LimitedBy::Picture));
    let outside = FitRequest { frame: 600, ..req };
    assert_eq!(fit::fit(&fx.project, fx.seq(), 1, &outside).expect("fitted").window, win(200.0, 240.0, 480.0), "no area over frame 600");
}

#[test]
fn t_m31_seeding_the_starting_window_stays_inside_the_area() {
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 900, 5_000)]);
    let mut set = one_plan(Vec::new());
    set.active_area = vec![area(0, 500, LETTERBOX)];
    fx.install(set);
    fx.apply(Write::SeedShots(SeedShots { plan: 1, mode: SeedMode::Starting, ..Default::default() }));
    let windows: Vec<(i64, Window)> = fx.plan(1).positions.iter().map(|p| (RATE.frame_at(p.at), p.window)).collect();
    assert_eq!(windows, vec![(0, win(960.0, 540.0, 816.0)), (100, win(960.0, 540.0, 816.0))]);
}

#[test]
fn t_m31_window_outside_active_runs_while_the_window_shows_bars() {
    let fx = letterboxed();
    let found = active_issues(&fx.project, fx.seq(), 1);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].code, "window_outside_active");
    assert_eq!(found[0].level, Level::Warning);
    assert_eq!(found[0].detail, json!({"position": 1, "runs": [[0, 300]], "area": [0.0, 132.0, 1920.0, 816.0]}));
    assert_eq!(with_code(&issues(&fx.project, fx.seq(), 1, 0, 1_000), "window_outside_active").len(), 1);
    let opts = MeasureOpts { from_frame: Some(299), to_frame: Some(301), series: true, ..Default::default() };
    let m = measure(&fx.project, fx.seq(), 1, &opts).expect("measured");
    let bars = m.series.expect("series").bar_pixels;
    assert_eq!(bars.len(), 2);
    assert!((bars[0] - 1_080.0 * 264.0 / 0.5625).abs() < 1e-6 && bars[1] == 0.0, "{bars:?}");
}

#[test]
fn t_m31_no_area_makes_a_plan_invalid_or_changes_its_data() {
    let fx = letterboxed();
    assert_eq!(validate(&fx.project, fx.seq()), Vec::new(), "I3 is the frame, whatever the area");
    let mut fx = Fx::new(1_000);
    fx.install(one_plan(vec![at25(1, 0, win(960.0, 540.0, 1080.0), Arrival::Cut)]));
    let before = fx.plan(1).clone();
    fx.apply(Write::SetActiveArea(SetActiveArea { set: vec![AreaSet { from_frame: 0, to_frame: 500, rect: LETTERBOX }], ..Default::default() }));
    assert_eq!(fx.plan(1), &before, "the plan is untouched");
    assert_eq!(fx.set().active_area, vec![area(0, 500, LETTERBOX)]);
}

#[test]
fn set_active_area_sets_and_clears_ranges() {
    let mut fx = Fx::new(1_000);
    fx.install(one_plan(Vec::new()));
    let set = |items: Vec<AreaSet>, clear: Vec<[i64; 2]>| Write::SetActiveArea(SetActiveArea { set: items, clear, ..Default::default() });
    fx.apply(set(vec![AreaSet { from_frame: 0, to_frame: 500, rect: LETTERBOX }], Vec::new()));
    fx.apply(set(Vec::new(), vec![[100, 200]]));
    assert_eq!(fx.set().active_area, vec![area(0, 100, LETTERBOX), area(200, 500, LETTERBOX)], "clearing splits the area");
    let pillar = [240.0, 0.0, 1440.0, 1080.0];
    fx.apply(set(vec![AreaSet { from_frame: 400, to_frame: 600, rect: pillar }], Vec::new()));
    let want = vec![area(0, 100, LETTERBOX), area(200, 400, LETTERBOX), area(400, 600, pillar)];
    assert_eq!(fx.set().active_area, want, "a new area replaces another over its frames");
    let again = accepted(fx.write(set(vec![AreaSet { from_frame: 400, to_frame: 600, rect: pillar }], Vec::new())));
    assert!(!again.changed, "setting what is set changes nothing");
    let stale = Write::SetActiveArea(SetActiveArea { clear: vec![[0, 1_000]], expect: Some("fnv1a64:0000000000000000".into()), ..Default::default() });
    let e = fx.refuse(stale);
    assert_eq!((e.code, e.detail["digest"].clone()), ("stale_plan", json!(data_digest(fx.set()))), "expect is the dataDigest");
}
