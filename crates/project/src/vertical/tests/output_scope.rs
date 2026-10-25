//! T-M27 (spec section 2.6): what the output digest of a part covers. Each change that makes the delivered file differ
//! changes it and turns `approved` into `outputChangedSinceApproval`: a cue's text inside the part, a cue's time across an
//! edge, the caption style or language, an effect parameter or a Lumetri value of a clip in range, an audio clip's gain in
//! range, a track's mixer setting, a transition in range, the media path (a relink), the plan's output size or caption
//! layout, the part's fades or metadata. Nothing outside it changes it: a cue or a clip outside the part, a marker, a
//! clip's name, another plan, an annotation, a box track, an attention mark, the plan's positions. Undo (the snapshot
//! before the change) restores `approved`.

use filmcraft_time::TimeRange;

use super::fixtures::*;
use super::support::win;
use crate::vertical::*;
use crate::{EffectKind, Marker, MarkerId, MarkerKind, MediaRef, ParamValue, TrackKind, Transition, TransitionAlign, TransitionId, find_effect};

/// Clips [0, 200) and [200, 600) on the first video track with a Lumetri on the first, a clip [400, 500) on the second, an
/// audio clip over everything, cues inside the part, across its out-edge and outside it; plans 9:16 and 4:5; part P01
/// [100, 250) approved in plan 1 with its current digests.
fn scoped() -> Fx {
    let mut fx = Fx::with_clips(&[(0, 200, 0), (200, 400, 5_000)]);
    fx.add_clip(1, 400, 100, 9_000);
    let audio = fx.project.make_track_item(fx.media, TrackKind::Audio, tk(0), TimeRange::new(tk(0), tk(600)), RATE).expect("the matte exists");
    fx.seq_mut().audio_tracks[0].items.push(audio);
    let first = fx.seq().video_tracks[0].items[0].id;
    fx.item_mut(first).effects.push(find_effect("lumetri").expect("Lumetri exists").instance());
    add_cues(&mut fx, &[(120, 150), (240, 260), (400, 420)]);
    let mut set = one_plan(vec![at25(1, 0, win(700.0, 540.0, 1080.0), Arrival::Cut), at25(2, 200, win(1100.0, 540.0, 1080.0), Arrival::Cut)]);
    let mut second = VerticalPlan::new(2, "4:5", OutputSize { width: 1080, height: 1350 });
    second.positions = vec![at25(1, 0, win(960.0, 540.0, 1080.0), Arrival::Cut)];
    second.next_position_id = 2;
    set.plans.push(second);
    set.next_plan_id = 3;
    set.parts = vec![part25(1, "P01", 100, 250)];
    set.next_part_id = 2;
    fx.install(set);
    let framing = range_digest(&fx.project, fx.seq(), fx.plan(1), 100, 250);
    let output = output_digest(&fx.project, fx.seq(), fx.plan(1), &fx.set().parts[0]);
    let by = "Dana Editor".to_string();
    let approval = PartApproval { part: 1, framing_digest: framing, output_digest: output, by, when: 1_790_000_000, note: String::new() };
    fx.plan_mut(1).part_approvals = vec![approval];
    fx
}

fn digest(fx: &Fx) -> String {
    output_digest(&fx.project, fx.seq(), fx.plan(1), &fx.set().parts[0])
}

/// Whether `edit` changes the output digest, and the part's status after it.
fn after(edit: impl FnOnce(&mut Fx)) -> (bool, Option<PartStatus>) {
    let mut fx = scoped();
    let before = digest(&fx);
    edit(&mut fx);
    (digest(&fx) != before, part_status(&fx.project, fx.seq(), 1, 1))
}

fn set_param(fx: &mut Fx, track: usize, item: usize, effect: &str, param: &str, value: ParamValue) {
    let id = fx.seq().video_tracks[track].items[item].id;
    let p = fx.item_mut(id).effect_mut(effect).and_then(|e| e.param_mut(param)).expect("the parameter exists");
    p.value = value;
}

const OUTPUT_CHANGED: (bool, Option<PartStatus>) = (true, Some(PartStatus::OutputChangedSinceApproval));
const UNCHANGED: (bool, Option<PartStatus>) = (false, Some(PartStatus::Approved));

#[test]
fn t_m27_the_scoped_fixture_starts_approved() {
    let fx = scoped();
    assert!(!digest(&fx).is_empty());
    assert_eq!(part_status(&fx.project, fx.seq(), 1, 1), Some(PartStatus::Approved));
}

#[test]
fn t_m27_captions_in_the_part_are_in_the_output_digest() {
    assert_eq!(after(|fx| fx.seq_mut().caption_tracks[0].captions[0].text = "changed".into()), OUTPUT_CHANGED, "a cue's text inside the part");
    assert_eq!(
        after(|fx| {
            let cue = &mut fx.seq_mut().caption_tracks[0].captions[1];
            cue.start = tk(245);
            cue.duration = tk(15);
        }),
        OUTPUT_CHANGED,
        "a cue's time across the out-edge"
    );
    assert_eq!(after(|fx| fx.seq_mut().caption_tracks[0].style.size = 60.0), OUTPUT_CHANGED, "the caption style");
    assert_eq!(after(|fx| fx.seq_mut().caption_tracks[0].language = "he".into()), OUTPUT_CHANGED, "the language");
    assert_eq!(after(|fx| fx.seq_mut().caption_tracks[0].enabled = false), OUTPUT_CHANGED, "the enabled flag");
    assert_eq!(after(|fx| fx.seq_mut().caption_tracks[0].captions[2].text = "changed".into()), UNCHANGED, "a cue outside the part");
}

#[test]
fn t_m27_items_transitions_media_and_the_mix_in_range_are_in_the_output_digest() {
    assert_eq!(after(|fx| set_param(fx, 0, 0, "motion", "scale", ParamValue::Float(110.0))), OUTPUT_CHANGED, "an effect parameter in range");
    assert_eq!(after(|fx| set_param(fx, 0, 0, "lumetri", "exposure", ParamValue::Float(0.5))), OUTPUT_CHANGED, "a Lumetri value in range");
    assert_eq!(after(|fx| fx.seq_mut().audio_tracks[0].items[0].gain_db = 3.0), OUTPUT_CHANGED, "an audio clip's gain in range");
    assert_eq!(after(|fx| fx.seq_mut().audio_tracks[0].volume_db = -3.0), OUTPUT_CHANGED, "a track's mixer setting");
    assert_eq!(after(|fx| fx.seq_mut().master_volume_db = -1.0), OUTPUT_CHANGED, "the master volume");
    assert_eq!(
        after(|fx| {
            let dissolve = crate::vtransition::find_transition("cross_dissolve", EffectKind::VideoTransition).expect("Cross Dissolve exists");
            let (a, b) = (fx.seq().video_tracks[0].items[0].id, fx.seq().video_tracks[0].items[1].id);
            fx.seq_mut().video_tracks[0].transitions.push(Transition {
                id: TransitionId(77),
                effect: dissolve.instance(),
                start: tk(190),
                duration: tk(20),
                from: Some(a),
                to: Some(b),
                align: TransitionAlign::CenterAtCut,
                reverse: false,
            });
        }),
        OUTPUT_CHANGED,
        "a transition in range"
    );
    assert_eq!(
        after(|fx| {
            let media = fx.media;
            if let Some(clip) = fx.project.item_mut(media).and_then(|i| i.as_media_mut()) {
                clip.media = MediaRef::File { path: "D:/media/relinked.mxf".into() };
            }
        }),
        OUTPUT_CHANGED,
        "the media path: a relink"
    );
    assert_eq!(after(|fx| set_param(fx, 1, 0, "motion", "scale", ParamValue::Float(110.0))), UNCHANGED, "a clip outside the part");
    assert_eq!(after(|fx| fx.seq_mut().video_tracks[0].items[0].name = "renamed".into()), UNCHANGED, "a clip's name");
}

#[test]
fn t_m27_the_plans_output_and_the_parts_delivery_settings_are_in_the_output_digest() {
    assert_eq!(after(|fx| fx.plan_mut(1).output = OutputSize { width: 720, height: 1280 }), OUTPUT_CHANGED, "the output size");
    assert_eq!(after(|fx| fx.plan_mut(1).captions.max_lines = 3), OUTPUT_CHANGED, "the caption layout");
    assert_eq!(after(|fx| fx.plan_mut(1).filter = Filter::Bilinear), OUTPUT_CHANGED, "the filter");
    assert_eq!(after(|fx| fx.plan_mut(1).fit_background = FitBackground::Black), OUTPUT_CHANGED, "the fit background");
    assert_eq!(after(|fx| fx.set_mut().parts[0].audio = Some(PartAudio { fade_in_frames: 0, fade_out_frames: 8 })), OUTPUT_CHANGED, "the part's fades");
    assert_eq!(after(|fx| fx.set_mut().part_audio = PartAudio { fade_in_frames: 6, fade_out_frames: 6 }), OUTPUT_CHANGED, "the set's fades the part uses");
    assert_eq!(after(|fx| fx.set_mut().parts[0].meta.title = "Dana walks".into()), OUTPUT_CHANGED, "the part's metadata");
}

#[test]
fn t_m27_what_never_changes_the_delivered_file_leaves_the_output_digest() {
    let marker = Marker {
        id: MarkerId(5),
        start: tk(120),
        duration: tk(0),
        name: "note".into(),
        comment: String::new(),
        kind: MarkerKind::Comment,
        color: crate::Label::Rose,
    };
    assert_eq!(after(|fx| fx.seq_mut().markers.push(marker)), UNCHANGED, "a marker");
    assert_eq!(after(|fx| fx.plan_mut(2).positions[0].window.cx = 900.0), UNCHANGED, "another plan");
    assert_eq!(after(|fx| fx.plan_mut(2).output = OutputSize { width: 1080, height: 1080 }), UNCHANGED, "another plan's output");
    assert_eq!(
        after(|fx| {
            fx.set_mut().annotations.push(Annotation {
                id: 1,
                kind: "speech".into(),
                from: tk(110),
                to: tk(140),
                label: String::new(),
                score: None,
                origin: Origin::Agent,
                modified_by: Writer::Agent,
            });
        }),
        UNCHANGED,
        "an annotation"
    );
    assert_eq!(
        after(|fx| {
            let keys = vec![KeyBox { at: tk(110), rect: [100.0, 100.0, 50.0, 50.0], facing: None, gone: false }];
            let track = BoxTrack { id: "face:1".into(), label: String::new(), kind: String::new(), keys, origin: Origin::Agent, modified_by: Writer::Agent };
            fx.set_mut().box_tracks.push(track);
        }),
        UNCHANGED,
        "a box track"
    );
    assert_eq!(
        after(|fx| fx.plan_mut(1).attention.push(AttentionMark { at: tk(130), by: "Dana Editor".into(), when: 1, note: String::new() })),
        UNCHANGED,
        "an attention mark: it is in neither digest"
    );
    let (output, status) = after(|fx| fx.plan_mut(1).positions[1].window.cx = 1_000.0);
    assert_eq!((output, status), (false, Some(PartStatus::ChangedSinceApproval)), "positions are the framing digest's");
}

#[test]
fn t_m27_undo_restores_approved() {
    let mut fx = scoped();
    let snapshot = fx.project.clone();
    fx.seq_mut().caption_tracks[0].captions[0].text = "changed".into();
    assert_eq!(part_status(&fx.project, fx.seq(), 1, 1), Some(PartStatus::OutputChangedSinceApproval));
    fx.project = snapshot;
    assert_eq!(part_status(&fx.project, fx.seq(), 1, 1), Some(PartStatus::Approved));
}
