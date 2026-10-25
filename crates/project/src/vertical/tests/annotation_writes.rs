//! Time annotations (spec section 4.9; I17): content-idempotent adds (an identical add returns the existing id),
//! updates, removes, `replace` of one kind inside a range, the 1/1000 grid of scores, the kind syntax, and `expect` as the
//! `dataDigest`.

use serde_json::json;

use super::fixtures::*;
use crate::vertical::*;

fn add(kind: &str, from: i64, to: i64, label: &str, score: Option<f64>) -> AnnotationAdd {
    AnnotationAdd { kind: kind.into(), from_frame: from, to_frame: to, label: label.into(), score }
}

fn adds(items: Vec<AnnotationAdd>) -> Write {
    Write::SetAnnotations(SetAnnotations { add: items, ..Default::default() })
}

/// (id, kind, from, to) of every annotation, in stored order.
fn listed(fx: &Fx) -> Vec<(u32, String, i64, i64)> {
    fx.set().annotations.iter().map(|a| (a.id, a.kind.clone(), RATE.frame_at(a.from), RATE.frame_at(a.to))).collect()
}

fn row(id: u32, kind: &str, from: i64, to: i64) -> (u32, String, i64, i64) {
    (id, kind.to_string(), from, to)
}

#[test]
fn annotation_adds_are_content_idempotent() {
    let mut fx = Fx::new(1_000);
    fx.install(one_plan(Vec::new()));
    let planned = fx.apply(adds(vec![add("speech", 10, 50, "char:dana", Some(0.5))]));
    assert_eq!(planned.ids, vec![1]);
    let a = &fx.set().annotations[0];
    assert_eq!((a.origin, a.modified_by, a.label.as_str(), a.score), (Origin::Manual, Writer::Human, "char:dana", Some(0.5)));
    let again = accepted(fx.write(adds(vec![add("speech", 10, 50, "char:dana", Some(0.5))])));
    assert!(!again.changed, "a retried add never duplicates a speech interval");
    assert_eq!(again.ids, vec![1]);
    let planned = fx.apply(adds(vec![add("speech", 10, 50, "char:dana", Some(0.75)), add("music", 5, 20, "", None)]));
    assert_eq!(planned.ids, vec![2, 3], "other content is another annotation");
    assert_eq!(listed(&fx), vec![row(3, "music", 5, 20), row(1, "speech", 10, 50), row(2, "speech", 10, 50)], "sorted by (from, id)");
    assert_eq!(fx.set().next_annotation_id, 4);
}

#[test]
fn updates_removes_and_the_grid_of_scores() {
    let mut fx = Fx::new(1_000);
    fx.install(one_plan(Vec::new()));
    fx.apply(adds(vec![add("speech", 10, 50, "char:dana", Some(0.123_4)), add("laughter", 60, 70, "", None)]));
    assert_eq!(fx.set().annotations[0].score, Some(0.123), "scores lie on the 1/1000 grid");
    let update = AnnotationUpdate { id: 1, label: Some("char:noa".into()), score: Some(None), to_frame: Some(55), ..Default::default() };
    fx.apply(Write::SetAnnotations(SetAnnotations { update: vec![update], ..Default::default() }));
    let a = &fx.set().annotations[0];
    assert_eq!((a.label.as_str(), a.score, RATE.frame_at(a.to)), ("char:noa", None, 55));
    fx.apply(Write::SetAnnotations(SetAnnotations { remove: vec![2], ..Default::default() }));
    assert_eq!(listed(&fx), vec![row(1, "speech", 10, 55)]);
    assert!(!accepted(fx.write(Write::SetAnnotations(SetAnnotations { remove: vec![2], ..Default::default() }))).changed, "removing what is gone");
    let missing = AnnotationUpdate { id: 9, label: Some("x".into()), ..Default::default() };
    let e = fx.refuse(Write::SetAnnotations(SetAnnotations { update: vec![missing], ..Default::default() }));
    assert_eq!((e.code, e.detail["annotation"].clone()), ("not_found", json!(9)));
}

#[test]
fn replace_swaps_every_annotation_of_one_kind_inside_a_range() {
    let mut fx = Fx::new(1_000);
    fx.install(one_plan(Vec::new()));
    fx.apply(adds(vec![add("speech", 10, 30, "", None), add("speech", 40, 60, "", None), add("speech", 90, 120, "", None), add("music", 20, 50, "", None)]));
    let replace = AnnotationReplace {
        kind: "speech".into(),
        from_frame: 0,
        to_frame: 100,
        set: vec![AnnotationItem { from_frame: 20, to_frame: 45, label: "char:dana".into(), score: None }],
    };
    let planned = fx.apply(Write::SetAnnotations(SetAnnotations { replace: Some(replace.clone()), ..Default::default() }));
    assert_eq!(planned.ids, Vec::<u32>::new(), "ids name the adds only");
    assert_eq!(
        listed(&fx),
        vec![row(4, "music", 20, 50), row(5, "speech", 20, 45), row(3, "speech", 90, 120)],
        "the speech inside [0, 100) is replaced; the one straddling its end and the music stay; sorted by (from, id)"
    );
    let again = accepted(fx.write(Write::SetAnnotations(SetAnnotations { replace: Some(replace), ..Default::default() })));
    assert!(!again.changed, "the same analysis again keeps the identical annotation and its id");
}

#[test]
fn kinds_and_track_ids_follow_their_syntax() {
    for kind in ["speech", "cue:sheet-1_x", "a", "0123456789abcdefghijklmnopqrstuv"] {
        assert!(valid_kind(kind), "{kind}");
    }
    for kind in ["", "Speech", "speech!", "has space", "0123456789abcdefghijklmnopqrstuvw", "דיבור"] {
        assert!(!valid_kind(kind), "{kind}");
    }
    let (longest, too_long) = ("x".repeat(64), "x".repeat(65));
    for id in ["char:dana", "face:3", "A.b-c_d:e", longest.as_str()] {
        assert!(valid_track_id(id), "{id}");
    }
    for id in ["", "bad id!", "דנה", too_long.as_str()] {
        assert!(!valid_track_id(id), "{id}");
    }
}

#[test]
fn expect_is_the_data_digest() {
    let mut fx = Fx::new(1_000);
    fx.install(one_plan(Vec::new()));
    let current = data_digest(fx.set());
    let guarded =
        |digest: &str| Write::SetAnnotations(SetAnnotations { add: vec![add("music", 0, 10, "", None)], expect: Some(digest.into()), ..Default::default() });
    let e = fx.refuse(guarded("fnv1a64:0000000000000000"));
    assert_eq!((e.code, e.detail["digest"].clone()), ("stale_plan", json!(current)));
    fx.apply(guarded(&current));
    assert_eq!(fx.set().annotations.len(), 1);
}
