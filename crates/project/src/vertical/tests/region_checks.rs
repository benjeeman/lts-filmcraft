//! T-M19 (spec section 4.7): a region is `in`, `sliced` or `out` of the window on each frame, `fit` frames count as in,
//! and the runs come out per mode: `keepIn` raises `region_cut` for sliced or out frames, `allOrNothing` raises
//! `region_sliced` for sliced frames, `keepOut` raises `region_visible` for in or sliced frames. `vertical.measure` reports
//! the counts, runs, worst frame and margins. And the region writes: content-idempotent adds, update, remove, the grid.

use serde_json::json;

use super::fixtures::*;
use super::support::win;
use crate::vertical::*;

/// The leftmost window until 100, the centred one until 200, a fit stretch until 300, the rightmost window to the end.
fn windows() -> Fx {
    let mut fx = Fx::new(400);
    fx.install(one_plan(vec![
        at25(1, 0, win(303.75, 540.0, 1080.0), Arrival::Cut),
        at25(2, 100, win(960.0, 540.0, 1080.0), Arrival::Cut),
        Position { layout: Layout::Fit, ..at25(3, 200, win(960.0, 540.0, 1080.0), Arrival::Cut) },
        at25(4, 300, win(1616.25, 540.0, 1080.0), Arrival::Cut),
    ]));
    fx
}

/// Inside the leftmost window only.
const LEFT: [f64; 4] = [100.0, 100.0, 200.0, 200.0];
/// Across the leftmost and the centred window's edges.
const EDGE: [f64; 4] = [500.0, 100.0, 300.0, 200.0];

fn run(from: i64, to: i64, state: RegionState, min_visible: f64) -> RegionRun {
    RegionRun { from_frame: from, to_frame: to, state, min_visible }
}

fn region(id: u32, from: i64, to: i64, rect: [f64; 4], mode: RegionMode) -> ProtectedRegion {
    ProtectedRegion { id, label: format!("r{id}"), from: tk(from), to: tk(to), rect, mode, origin: Origin::Manual, modified_by: Writer::Human }
}

#[test]
fn t_m19_in_sliced_and_out_runs_and_fit_frames_count_as_in() {
    let fx = windows();
    let (plan, r) = (fx.plan(1), fx.r());
    assert_eq!(
        region_runs(plan, &r, LEFT, 0, 400),
        vec![
            run(0, 100, RegionState::In, 1.0),
            run(100, 200, RegionState::Out, 0.0),
            run(200, 300, RegionState::In, 1.0),
            run(300, 400, RegionState::Out, 0.0)
        ]
    );
    assert_eq!(
        region_runs(plan, &r, EDGE, 0, 400),
        vec![run(0, 200, RegionState::Sliced, 107.5 / 300.0), run(200, 300, RegionState::In, 1.0), run(300, 400, RegionState::Out, 0.0)],
        "consecutive sliced frames make one run with their smallest visible fraction"
    );
    assert_eq!(visible_fraction(plan, &r, 150, EDGE), 143.75 / 300.0);
    assert_eq!(visible_fraction(plan, &r, 250, [0.0, 0.0, 10.0, 10.0]), 1.0, "a fit frame shows the whole picture");
    assert_eq!(region_state(plan, &r, 350, LEFT), RegionState::Out);
    assert_eq!(region_runs(plan, &r, LEFT, 150, 250), vec![run(150, 200, RegionState::Out, 0.0), run(200, 250, RegionState::In, 1.0)]);
}

#[test]
fn t_m19_each_mode_raises_its_own_warning_over_its_own_frames() {
    let mut fx = windows();
    fx.set_mut().regions = vec![
        region(1, 0, 400, LEFT, RegionMode::KeepIn),
        region(2, 0, 400, EDGE, RegionMode::AllOrNothing),
        region(3, 0, 400, LEFT, RegionMode::KeepOut),
        region(4, 150, 250, LEFT, RegionMode::KeepIn),
    ];
    let found = region_issues(&fx.project, fx.seq(), 1);
    let detail = |code: &str, id: u32| found.iter().find(|i| i.code == code && i.detail["region"] == json!(id)).map(|i| (i.level, i.detail["runs"].clone()));
    let runs = |list: &[(i64, i64, &str, f64)]| {
        json!(list.iter().map(|&(a, b, s, v)| json!({"fromFrame": a, "toFrame": b, "state": s, "minVisible": v})).collect::<Vec<_>>())
    };
    assert_eq!(detail("region_cut", 1), Some((Level::Warning, runs(&[(100, 200, "out", 0.0), (300, 400, "out", 0.0)]))));
    assert_eq!(detail("region_sliced", 2), Some((Level::Warning, runs(&[(0, 200, "sliced", 107.5 / 300.0)]))));
    assert_eq!(detail("region_visible", 3), Some((Level::Warning, runs(&[(0, 100, "in", 1.0), (200, 300, "in", 1.0)]))));
    assert_eq!(detail("region_cut", 4), Some((Level::Warning, runs(&[(150, 200, "out", 0.0)]))), "only the region's own frames");
    assert_eq!(found.len(), 4, "{found:?}");
    // the issues of the plan carry them too
    assert_eq!(with_code(&issues(&fx.project, fx.seq(), 1, 0, 400), "region_visible").len(), 1);
}

#[test]
fn t_m19_measure_reports_counts_runs_the_worst_frame_and_margins() {
    let mut fx = windows();
    fx.set_mut().regions = vec![region(1, 0, 400, LEFT, RegionMode::KeepIn), region(2, 0, 400, LEFT, RegionMode::KeepOut)];
    let m = measure(&fx.project, fx.seq(), 1, &MeasureOpts { protected: true, ..Default::default() }).expect("measured");
    let keep_in = m.regions.iter().find(|r| r.region == Some(1)).expect("region 1");
    assert_eq!((keep_in.frames_in, keep_in.frames_sliced, keep_in.frames_out), (200, 0, 200));
    assert_eq!(keep_in.runs.len(), 4);
    assert_eq!(keep_in.worst_frame, Some(100), "the earliest frame with the smallest visible fraction");
    let margins = keep_in.margins.expect("margins at the worst frame");
    let expected = [-988.888_888_888_888_9, 177.777_777_777_777_77, 1_713.333_333_333_333_3, 1_386.666_666_666_666_7];
    for (got, want) in margins.iter().zip(expected) {
        assert!((got - want).abs() < 1e-9, "{margins:?}");
    }
    let keep_out = m.regions.iter().find(|r| r.region == Some(2)).expect("region 2");
    assert_eq!(keep_out.worst_frame, Some(0), "for keepOut the worst frame shows the most of it");
    assert!(!with_code(&m.findings, "region_cut").is_empty() && !with_code(&m.findings, "region_visible").is_empty());
}

#[test]
fn region_adds_are_content_idempotent_and_rects_lie_on_the_grid() {
    let mut fx = windows();
    let logo = RegionAdd { label: "logo".into(), from_frame: 0, to_frame: 400, rect: [1700.0, 40.0, 180.0, 90.0], mode: RegionMode::AllOrNothing };
    let add = |items: Vec<RegionAdd>| Write::SetRegions(SetRegions { add: items, ..Default::default() });
    let planned = fx.apply(add(vec![logo.clone()]));
    assert_eq!(planned.ids, vec![1]);
    let again = accepted(fx.write(add(vec![logo.clone()])));
    assert!(!again.changed);
    assert_eq!(again.ids, vec![1], "an identical add returns the existing id");
    let other = RegionAdd { rect: [1600.0, 40.0, 180.0, 90.0], ..logo.clone() };
    assert_eq!(fx.apply(add(vec![other])).ids, vec![2], "labels need not be unique");
    let r1 = &fx.set().regions[0];
    assert_eq!((r1.origin, r1.modified_by), (Origin::Manual, Writer::Human));
    // off-grid values are stored as q(v)
    let off = RegionAdd { label: "super".into(), from_frame: 10, to_frame: 20, rect: [100.0004, 50.0006, 200.0, 100.0], mode: RegionMode::KeepIn };
    let planned = fx.apply(add(vec![off]));
    let kept = fx.set().regions.iter().find(|r| Some(r.id) == planned.ids.first().copied()).expect("the new region");
    assert_eq!(kept.rect, [100.0, 50.001, 200.0, 100.0]);
    // update and remove by id
    let update = RegionUpdate { region: 1, mode: Some(RegionMode::KeepOut), ..Default::default() };
    fx.apply(Write::SetRegions(SetRegions { update: vec![update], remove: vec![2], ..Default::default() }));
    let ids: Vec<(u32, RegionMode)> = fx.set().regions.iter().map(|r| (r.id, r.mode)).collect();
    assert_eq!(ids, vec![(1, RegionMode::KeepOut), (3, RegionMode::KeepIn)]);
    // a rect outside the frame or with no area is refused
    let outside = RegionAdd { rect: [1900.0, 0.0, 100.0, 100.0], ..logo.clone() };
    assert_eq!(fx.refuse(add(vec![outside])).code, "outside_picture");
    let flat = RegionAdd { rect: [100.0, 100.0, 0.0, 100.0], ..logo };
    assert_eq!(fx.refuse(add(vec![flat])).code, "bad_number");
}
