//! T-M29 (spec section 4.6, the model side): the raw properties of part edges (cues on screen across an edge, the
//! nearest cue starts and ends on each side, the nearest picture wall, a dissolve over the edge, the annotations covering
//! it and the nearest annotation edges, durations, the gaps and overlaps between consecutive parts), and every edge finding
//! only for the threshold the caller passes. A cue strictly across an edge is `part_boundary_in_cue`; one ending exactly
//! on it is not. Audio levels, the audio onset and edge lumas are the engine's (M4).

use serde_json::json;

use super::fixtures::*;
use super::support::win;
use crate::vertical::*;
use crate::{EffectKind, Transition, TransitionAlign, TransitionId};

/// An edit wall at 100 and a manual wall at 302; parts P01 [50, 300), P02 [300, 600), P03 [580, 800) (overlapping P02 by
/// 20 frames) and P04 [850, 950) (50 frames after P03); cues [40, 60) across P01's in-edge, [280, 300) ending on its
/// out-edge and [590, 610) across P02's out-edge; a dissolve over [290, 310); speech over [290, 320), music over [840, 900).
fn edges() -> Fx {
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 900, 5_000)]);
    let mut set = one_plan(vec![at25(1, 0, win(700.0, 540.0, 1080.0), Arrival::Cut), at25(2, 100, win(1100.0, 540.0, 1080.0), Arrival::Cut)]);
    set.walls.stored = vec![stored(302, WallOrigin::Manual, None)];
    set.parts = vec![part25(1, "P01", 50, 300), part25(2, "P02", 300, 600), part25(3, "P03", 580, 800), part25(4, "P04", 850, 950)];
    set.next_part_id = 5;
    let annotation = |id: u32, kind: &str, from: i64, to: i64| Annotation {
        id,
        kind: kind.into(),
        from: tk(from),
        to: tk(to),
        label: String::new(),
        score: None,
        origin: Origin::Agent,
        modified_by: Writer::Agent,
    };
    set.annotations = vec![annotation(1, "speech", 290, 320), annotation(2, "music", 840, 900)];
    set.next_annotation_id = 3;
    fx.install(set);
    add_cues(&mut fx, &[(40, 60), (280, 300), (590, 610)]);
    let clip = fx.seq().video_tracks[0].items[1].id;
    let dissolve = crate::vtransition::find_transition("cross_dissolve", EffectKind::VideoTransition).expect("Cross Dissolve exists");
    fx.seq_mut().video_tracks[0].transitions.push(Transition {
        id: TransitionId(77),
        effect: dissolve.instance(),
        start: tk(290),
        duration: tk(20),
        from: Some(clip),
        to: None,
        align: TransitionAlign::CenterAtCut,
        reverse: false,
    });
    fx
}

#[test]
fn t_m29_the_raw_properties_of_a_parts_edges() {
    let fx = edges();
    let facts = edge_facts(&fx.project, fx.seq(), Some(1), 1, 250).expect("part 1");
    assert_eq!((facts.part, facts.name.as_str(), facts.from, facts.to, facts.duration), (1, "P01", 50, 300, 250));
    assert_eq!((facts.first_window, facts.last_window), (Some(win(700.0, 540.0, 1080.0)), Some(win(1100.0, 540.0, 1080.0))));
    let track = fx.seq().caption_tracks[0].id.0;
    let edge_in = &facts.in_edge;
    assert_eq!(edge_in.frame, 50);
    assert_eq!(edge_in.in_cue, vec![CueStraddle { track, cue: 1, before: 10, after: 10 }]);
    assert_eq!(edge_in.cue_edges, CueDistances { start_before: Some(-10), start_after: Some(230), end_before: None, end_after: Some(10) });
    assert_eq!(edge_in.wall, Some(WallDistance { frame: 100, distance: 50, sources: vec![WallSource::Edit] }), "part-only walls are not cuts");
    assert_eq!((edge_in.dissolve, edge_in.annotations.clone()), (None, vec![]));
    let edge_out = &facts.out_edge;
    assert_eq!(edge_out.frame, 300);
    assert!(edge_out.in_cue.is_empty(), "a cue ending exactly on the edge is not across it");
    assert_eq!(edge_out.cue_edges, CueDistances { start_before: Some(-20), start_after: Some(290), end_before: Some(0), end_after: Some(310) });
    assert_eq!(edge_out.wall, Some(WallDistance { frame: 302, distance: 2, sources: vec![WallSource::Manual] }));
    let clip_track = fx.seq().video_tracks[0].id.0;
    assert_eq!(edge_out.dissolve, Some(Dissolve { track: clip_track, transition: 77, from: 290, to: 310 }));
    assert_eq!(edge_out.annotations, vec![1], "the speech annotation covers frame 300");
    assert!(edge_out.nearest.iter().any(|n| n.kind == "speech" && n.annotation == 1 && n.frame == 290 && n.distance == -10), "{:?}", edge_out.nearest);
    assert_eq!(edge_facts(&fx.project, fx.seq(), Some(1), 9, 250), None, "no part 9");
}

#[test]
fn t_m29_gaps_and_overlaps_between_consecutive_parts() {
    let fx = edges();
    let edges = part_edges(&fx.project, fx.seq(), None, &[], 250, &EdgeThresholds::default());
    assert_eq!(edges.parts.len(), 4);
    assert_eq!(
        edges.gaps,
        vec![PartGap { before: 1, after: 2, frames: 0 }, PartGap { before: 2, after: 3, frames: -20 }, PartGap { before: 3, after: 4, frames: 50 }]
    );
    assert!(edges.findings.is_empty(), "no threshold, no finding: {:?}", edges.findings);
    let durations: Vec<i64> = edges.parts.iter().map(|p| p.duration).collect();
    assert_eq!(durations, vec![250, 300, 220, 100]);
}

/// The findings of `code` for `thresholds`, as (part, frame) pairs.
fn found(fx: &Fx, thresholds: EdgeThresholds, code: &str) -> Vec<(i64, i64)> {
    let edges = part_edges(&fx.project, fx.seq(), Some(1), &[], 250, &thresholds);
    let pair = |i: &&Issue| (i.detail["part"].as_i64().unwrap_or(-1), i.detail["frame"].as_i64().unwrap_or(-1));
    let mut pairs: Vec<(i64, i64)> = with_code(&edges.findings, code).iter().map(pair).collect();
    pairs.sort_unstable();
    pairs
}

#[test]
fn t_m29_every_finding_appears_only_for_its_threshold() {
    let fx = edges();
    let none = EdgeThresholds::default();
    assert_eq!(found(&fx, EdgeThresholds { edge_in_cue: true, ..none.clone() }, "part_boundary_in_cue"), vec![(1, 50), (2, 600)]);
    assert_eq!(found(&fx, EdgeThresholds { near_cut_frames: Some(3), ..none.clone() }, "part_boundary_near_cut"), vec![(1, 300), (2, 300)]);
    assert!(found(&fx, EdgeThresholds { near_cut_frames: Some(1), ..none.clone() }, "part_boundary_near_cut").is_empty());
    assert_eq!(found(&fx, EdgeThresholds { edge_in_dissolve: true, ..none.clone() }, "part_boundary_in_dissolve"), vec![(1, 300), (2, 300)]);
    let speech = EdgeThresholds { edge_annotation_kinds: vec!["speech".into()], ..none.clone() };
    assert_eq!(found(&fx, speech, "part_boundary_in_annotation"), vec![(1, 300), (2, 300)]);
    let music = EdgeThresholds { edge_annotation_kinds: vec!["music".into()], ..none.clone() };
    assert_eq!(found(&fx, music, "part_boundary_in_annotation"), vec![(4, 850)]);
    let lengths = EdgeThresholds { min_part_frames: Some(200), max_part_frames: Some(280), ..none.clone() };
    let edges = part_edges(&fx.project, fx.seq(), None, &[], 250, &lengths);
    let mut parts: Vec<serde_json::Value> = with_code(&edges.findings, "part_duration").iter().map(|i| i.detail["part"].clone()).collect();
    parts.sort_by_key(|p| p.as_i64());
    assert_eq!(parts, vec![json!(2), json!(4)], "P02 is longer than 280 frames, P04 shorter than 200");
    let spacing = EdgeThresholds { max_gap_frames: Some(40), overlap: Some(false), ..none };
    let edges = part_edges(&fx.project, fx.seq(), None, &[], 250, &spacing);
    let gap = with_code(&edges.findings, "parts_gap");
    let overlap = with_code(&edges.findings, "parts_overlap");
    assert_eq!((gap.len(), overlap.len()), (1, 1));
    assert_eq!(gap[0].detail, json!({"before": 3, "after": 4, "frames": 50}));
    assert_eq!(overlap[0].detail, json!({"before": 2, "after": 3, "frames": 20}));
    assert!(edges.findings.iter().all(|i| i.level == Level::Warning));
}
