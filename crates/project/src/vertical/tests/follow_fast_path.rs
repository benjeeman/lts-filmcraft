//! T-M15, the hook's fast path (spec section 4.8): two digests per sequence. Audio and marker edits leave both and take the
//! fast path; a Motion scale edit and a Crop zoom move no frame and flag `PictureChanged` on the positions whose frames show
//! the clip; an edit inside a nest flags the outer plan; the layout digest follows the picture layout and nothing else, the
//! framing digest Motion, time remapping and the geometry effects and nothing else; and digests the engine caches per
//! snapshot give the same result as digests computed in the call. Synthetic data only.

use filmcraft_time::{FrameRate, Tick, TimeRange};

use super::fixtures::{Fx, RATE, add_cues, at25, one_plan, tk};
use super::hook_fixtures::*;
use super::support::{digest_ok, mv, win};
use crate::vertical::*;
use crate::{ClipId, ItemId, Label, Marker, MarkerId, MarkerKind, ParamValue, Project, TrackKind, Transition, TransitionAlign, TransitionId, find_effect};

/// Clips A [0, 150) and B [150, 300); positions #1 and #2 frame A (spans [0, 31) and [31, 150)), #3 and #4 frame B.
fn two_clips() -> Fx {
    let mut fx = Fx::with_clips(&[(0, 150, 0), (150, 150, 10_000)]);
    fx.install(one_plan(vec![
        at25(1, 0, win(600.0, 540.0, 1080.0), Arrival::Cut),
        at25(2, 50, win(800.0, 540.0, 1080.0), mv(20, Curve::Linear)),
        at25(3, 150, win(1200.0, 540.0, 1080.0), Arrival::Cut),
        at25(4, 220, win(1000.0, 540.0, 1080.0), mv(30, Curve::Linear)),
    ]));
    fx
}

/// The ids of the plan's positions flagged `PictureChanged`.
fn picture_changed(plan: &VerticalPlan) -> Vec<u32> {
    plan.positions.iter().filter(|p| p.flags.contains(&Flag::PictureChanged)).map(|p| p.id).collect()
}

/// The layout and framing digests of sequence `seq` of `project`.
fn both(project: &Project, seq: ItemId) -> (String, String) {
    let s = seq_of(project, seq);
    (layout_digest(project, s), framing_digest(project, s))
}

/// A comment marker at frame 40.
fn marker(id: u64) -> Marker {
    Marker { id: MarkerId(id), start: tk(40), duration: Tick(0), name: "m".into(), comment: String::new(), kind: MarkerKind::Comment, color: Label::Rose }
}

/// Puts a clip of `media` on the first audio track of sequence `seq`, from frame 0, `frames` long.
fn add_audio(project: &mut Project, seq: ItemId, media: ItemId, frames: i64) {
    let item = project.make_track_item(media, TrackKind::Audio, tk(0), TimeRange::new(tk(0), tk(frames)), RATE).expect("the media exists");
    project.sequence_mut(seq).expect("the sequence").audio_tracks[0].items.push(item);
}

#[test]
fn t_m15_audio_and_marker_edits_take_the_fast_path() {
    let fx = two_clips();
    let before = fx.project.clone();
    let mut after = edited(&before, |p| {
        add_audio(p, fx.seq, fx.media, 300);
        p.sequence_mut(fx.seq).expect("the sequence").markers.push(marker(9_000));
    });
    assert_eq!(both(&before, fx.seq), both(&after, fx.seq), "audio and markers show no picture");
    let report = follow(&before, &mut after);
    assert!(report.is_empty(), "nothing to follow");
    assert_eq!(set_in(&after, fx.seq), set_in(&before, fx.seq));
}

#[test]
fn t_m15_a_motion_scale_edit_flags_the_positions_that_show_the_clip() {
    let fx = two_clips();
    let b = clip_ids(&fx.project, fx.seq)[1];
    let before = fx.project.clone();
    let mut after = edited(&before, |p| set_param(clip_mut(p, fx.seq, b), "motion", "scale", ParamValue::Float(110.0)));
    let ((layout0, framing0), (layout1, framing1)) = (both(&before, fx.seq), both(&after, fx.seq));
    assert_eq!(layout0, layout1, "a Motion edit moves no frame");
    assert_ne!(framing0, framing1);
    let report = follow(&before, &mut after);
    let (old, new) = (plan_in(&before, fx.seq, 1), plan_in(&after, fx.seq, 1));
    assert_eq!(picture_changed(new), vec![3, 4], "the positions whose frames show B");
    assert_eq!(frames_of(new), frames_of(old), "nothing moves");
    let pr = plan_report(&report, fx.seq, 1);
    assert_eq!((pr.flagged, pr.moved, pr.removed, pr.added), (vec![3, 4], vec![], vec![], vec![]));
}

#[test]
fn t_m15_a_crop_zoom_flags_the_positions_that_show_the_clip() {
    let fx = two_clips();
    let a = clip_ids(&fx.project, fx.seq)[0];
    let before = fx.project.clone();
    let mut after = edited(&before, |p| {
        let mut crop = find_effect("crop").expect("Crop exists").instance();
        crop.param_mut("left").expect("left").value = ParamValue::Float(10.0);
        crop.param_mut("zoom").expect("zoom").value = ParamValue::Bool(true);
        clip_mut(p, fx.seq, a).effects.push(crop);
    });
    assert_eq!(both(&before, fx.seq).0, both(&after, fx.seq).0, "a Crop moves no frame");
    let report = follow(&before, &mut after);
    assert_eq!(picture_changed(plan_in(&after, fx.seq, 1)), vec![1, 2], "the positions whose frames show A");
    assert_eq!(plan_report(&report, fx.seq, 1).flagged, vec![1, 2]);
}

#[test]
fn t_m15_an_edit_inside_a_nest_flags_the_outer_plan() {
    let mut fx = Fx::with_clips(&[]);
    let (nest, inner) = add_nest(&mut fx, 300, 0);
    let item = fx.make_item(nest, tk(0), 150, 0);
    fx.put(0, item);
    let media = fx.media;
    let clip = fx.make_item(media, tk(150), 150, 20_000);
    fx.put(0, clip);
    fx.install(one_plan(vec![
        at25(1, 0, win(600.0, 540.0, 1080.0), Arrival::Cut),
        at25(2, 60, win(800.0, 540.0, 1080.0), Arrival::Cut),
        at25(3, 150, win(1200.0, 540.0, 1080.0), Arrival::Cut),
        at25(4, 200, win(1000.0, 540.0, 1080.0), Arrival::Cut),
    ]));
    let before = fx.project.clone();
    let mut after = edited(&before, |p| set_param(clip_mut(p, nest, inner), "motion", "scale", ParamValue::Float(120.0)));
    assert_ne!(both(&before, fx.seq).0, both(&after, fx.seq).0, "the parent's layout digest holds both digests of its nests");
    let report = follow(&before, &mut after);
    let new = plan_in(&after, fx.seq, 1);
    assert_eq!(picture_changed(new), vec![1, 2], "the positions whose frames show the nest");
    assert_eq!(frames_of(new), vec![0, 60, 150, 200], "nothing moves");
    assert_eq!(plan_report(&report, fx.seq, 1).flagged, vec![1, 2]);
}

/// Clips A [0, 100) and B [100, 200) on track 0, and at 200 a clip of a nested sequence (whose own clip it returns).
fn digest_base() -> (Fx, ItemId, ClipId) {
    let mut fx = Fx::with_clips(&[(0, 100, 0), (100, 100, 10_000)]);
    let (nest, inner) = add_nest(&mut fx, 100, 0);
    let item = fx.make_item(nest, tk(200), 50, 0);
    fx.put(0, item);
    fx.install(one_plan(vec![at25(1, 0, win(600.0, 540.0, 1080.0), Arrival::Cut)]));
    (fx, nest, inner)
}

#[test]
fn t_m15_the_layout_digest_follows_the_picture_layout_and_nothing_else() {
    let (mut fx, nest, inner) = digest_base();
    let other = add_matte(&mut fx.project, "insert");
    let (seq, media, base) = (fx.seq, fx.media, fx.project.clone());
    let ids = clip_ids(&base, seq);
    let (a, b) = (ids[0], ids[1]);
    let layout = |p: &Project| layout_digest(p, seq_of(p, seq));
    let d0 = layout(&base);
    assert!(digest_ok(&d0), "{d0}");
    assert_eq!(layout(&base.clone()), d0, "a pure function of the content");
    let dissolve = Transition {
        id: TransitionId(9_001),
        effect: find_effect("cross_dissolve").expect("Cross Dissolve exists").instance(),
        start: tk(95),
        duration: tk(10),
        from: Some(a),
        to: Some(b),
        align: TransitionAlign::CenterAtCut,
        reverse: false,
    };
    let second_track = |p: &mut Project| {
        let item = p.make_track_item(other, TrackKind::Video, tk(20), TimeRange::new(tk(0), tk(30)), RATE).expect("the media exists");
        video_mut(p, seq, 1).items.push(item);
    };
    let changes: Vec<(&str, Project)> = vec![
        ("a clip starts a frame later", edited(&base, |p| clip_mut(p, seq, b).start = tk(101))),
        ("a clip is shorter", edited(&base, |p| clip_mut(p, seq, a).duration = tk(99))),
        ("a clip's source in", edited(&base, |p| clip_mut(p, seq, a).source_in = tk(5))),
        ("a speed", edited(&base, |p| clip_mut(p, seq, a).speed = 2.0)),
        ("reverse", edited(&base, |p| clip_mut(p, seq, a).reverse = true)),
        ("a frame hold", edited(&base, |p| clip_mut(p, seq, a).frame_hold = Some(tk(10)))),
        ("scale to frame size", edited(&base, |p| clip_mut(p, seq, a).scale_to_frame = true)),
        ("a disabled clip", edited(&base, |p| clip_mut(p, seq, a).enabled = false)),
        ("a disabled track", edited(&base, |p| video_mut(p, seq, 0).enabled = false)),
        ("a dissolve", edited(&base, |p| video_mut(p, seq, 0).transitions.push(dissolve.clone()))),
        ("the frame size", edited(&base, |p| p.sequence_mut(seq).expect("the sequence").settings.width = 3840)),
        ("the frame rate", edited(&base, |p| p.sequence_mut(seq).expect("the sequence").settings.frame_rate = FrameRate::FPS_50)),
        ("a clip on the second track", edited(&base, second_track)),
        ("an edit inside the nest", edited(&base, |p| clip_mut(p, nest, inner).source_in = tk(10))),
        ("a Motion edit inside the nest", edited(&base, |p| set_param(clip_mut(p, nest, inner), "motion", "scale", ParamValue::Float(120.0)))),
    ];
    for (what, project) in &changes {
        assert_ne!(layout(project), d0, "{what} changes the layout digest");
    }
    let mut cues = Fx { project: base.clone(), seq, media };
    add_cues(&mut cues, &[(10, 40)]);
    let same: Vec<(&str, Project)> = vec![
        ("an audio clip", edited(&base, |p| add_audio(p, seq, media, 200))),
        ("a marker", edited(&base, |p| p.sequence_mut(seq).expect("the sequence").markers.push(marker(9_002)))),
        ("captions", cues.project),
        ("a clip's name", edited(&base, |p| clip_mut(p, seq, a).name = "renamed".into())),
        ("a Motion edit", edited(&base, |p| set_param(clip_mut(p, seq, a), "motion", "scale", ParamValue::Float(120.0)))),
        ("a Crop", edited(&base, |p| clip_mut(p, seq, a).effects.push(find_effect("crop").expect("Crop exists").instance()))),
        ("the vertical data", edited(&base, |p| p.sequence_mut(seq).expect("the sequence").vertical = None)),
    ];
    for (what, project) in &same {
        assert_eq!(layout(project), d0, "{what} leaves the layout digest");
    }
}

#[test]
fn t_m15_the_framing_digest_follows_motion_and_the_geometry_effects_and_nothing_else() {
    let (fx, _, _) = digest_base();
    let (seq, base) = (fx.seq, fx.project.clone());
    let a = clip_ids(&base, seq)[0];
    let framing = |p: &Project| framing_digest(p, seq_of(p, seq));
    let d0 = framing(&base);
    assert!(digest_ok(&d0), "{d0}");
    let with = |effect: &str, enabled: bool| {
        edited(&base, |p| {
            let mut e = find_effect(effect).expect("the effect exists").instance();
            e.enabled = enabled;
            clip_mut(p, seq, a).effects.push(e);
        })
    };
    let changes: Vec<(&str, Project)> = vec![
        ("a Motion scale", edited(&base, |p| set_param(clip_mut(p, seq, a), "motion", "scale", ParamValue::Float(110.0)))),
        ("Motion off", edited(&base, |p| clip_mut(p, seq, a).effect_mut("motion").expect("Motion").enabled = false)),
        ("time remapping", edited(&base, |p| set_param(clip_mut(p, seq, a), "time_remap", "speed", ParamValue::Float(150.0)))),
        ("a Crop", with("crop", true)),
        ("a Transform effect", with("horizontal_flip", true)),
        ("a disabled track", edited(&base, |p| video_mut(p, seq, 0).enabled = false)),
    ];
    for (what, project) in &changes {
        assert_ne!(framing(project), d0, "{what} changes the framing digest");
    }
    let same: Vec<(&str, Project)> = vec![
        ("a disabled Crop", with("crop", false)),
        ("a blur", with("gaussian_blur", true)),
        ("the opacity", edited(&base, |p| set_param(clip_mut(p, seq, a), "opacity", "opacity", ParamValue::Float(50.0)))),
        ("a clip's name", edited(&base, |p| clip_mut(p, seq, a).name = "renamed".into())),
        ("a later start", edited(&base, |p| clip_mut(p, seq, a).start = tk(1))),
    ];
    for (what, project) in &same {
        assert_eq!(framing(project), d0, "{what} leaves the framing digest");
    }
}

#[test]
fn t_m15_picture_digests_cover_the_planned_sequences() {
    let (fx, nest, _) = digest_base();
    let digests = picture_digests(&fx.project);
    let entry = digests.sequences.get(&fx.seq).expect("the planned sequence has digests");
    assert_eq!((entry.layout.clone(), entry.framing.clone()), both(&fx.project, fx.seq));
    assert!(!digests.sequences.contains_key(&nest), "a sequence without a vertical set needs none");
}

#[test]
fn t_m15_cached_digests_give_the_same_result_as_digests_computed_in_the_call() {
    let mut fx = Fx::with_clips(&[(0, 400, 0)]);
    fx.install(one_plan(vec![at25(1, 100, win(812.4, 540.0, 1080.0), Arrival::Cut), at25(2, 150, win(960.0, 540.0, 1080.0), mv(50, Curve::EaseInOut))]));
    let before = fx.project.clone();
    let edit = edited(&before, |p| ripple_delete(p, fx.seq, 120, 130));
    let (mut computed, mut cached) = (edit.clone(), edit.clone());
    let report = follow(&before, &mut computed);
    let (old, new) = (picture_digests(&before), picture_digests(&cached));
    let again = follow_picture_with(&before, &old, &mut cached, &new, PERSON).expect("a person's edit is followed");
    assert_eq!(report, again);
    assert!(same_project(&computed, &cached), "the same project either way");
    assert_ne!(set_in(&computed, fx.seq), set_in(&before, fx.seq), "the ripple moved the plan");
    assert_eq!(picture_digests(&cached), new, "the hook changes no picture digest of its own result");
}
