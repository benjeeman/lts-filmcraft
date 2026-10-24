//! T-M24, the round-trip half: every stored type round-trips with camelCase names, omitted defaults and exact floats, and
//! `Arrival` and `Curve` have the exact JSON shapes of spec section 2.4. (The bounds half is M2's.)

use serde::Serialize;
use serde::de::DeserializeOwned;
use std::fmt::Debug;

use super::support::*;
use crate::vertical::*;

/// Prints `value`, reads it back, checks both are equal and the text is stable, and returns the text.
fn round_trip<T: Serialize + DeserializeOwned + PartialEq + Debug>(value: &T) -> String {
    let text = serde_json::to_string(value).expect("prints");
    let back: T = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{text} does not read back: {e}"));
    assert_eq!(&back, value, "{text}");
    assert_eq!(serde_json::to_string(&back).expect("prints"), text);
    text
}

fn has_key(text: &str, key: &str) -> bool {
    text.contains(&format!("\"{key}\":"))
}

#[test]
fn t_m24_arrival_has_the_exact_json_shapes() {
    let curve = Curve::EaseInOut;
    let cases = [
        (Arrival::Cut, r#"{"kind":"cut"}"#),
        (mv(40, curve), r#"{"kind":"move","moveFrames":40,"curve":{"type":"easeInOut"}}"#),
        (mv_split(20, curve, 30, 50), r#"{"kind":"move","moveFrames":20,"curve":{"type":"easeInOut"},"splitOf":{"offset":30,"total":50}}"#),
    ];
    for (arrival, text) in cases {
        assert_eq!(round_trip(&arrival), text);
        assert_eq!(serde_json::from_str::<Arrival>(text).ok(), Some(arrival));
    }
    // snake_case names are not read as the fields
    assert!(serde_json::from_str::<Arrival>(r#"{"kind":"move","move_frames":40,"curve":{"type":"linear"}}"#).is_err());
}

#[test]
fn t_m24_curve_has_the_exact_json_shapes() {
    let curves = [
        (Curve::Linear, r#"{"type":"linear"}"#),
        (Curve::EaseIn, r#"{"type":"easeIn"}"#),
        (Curve::EaseOut, r#"{"type":"easeOut"}"#),
        (Curve::EaseInOut, r#"{"type":"easeInOut"}"#),
        (Curve::Speeds { start: 0.0, end: 1.75 }, r#"{"type":"speeds","start":0.0,"end":1.75}"#),
        (Curve::Bezier { x1: 0.25, y1: 0.1, x2: 0.25, y2: 1.0 }, r#"{"type":"bezier","x1":0.25,"y1":0.1,"x2":0.25,"y2":1.0}"#),
        (Curve::Ramp { r_in: 0.2, r_out: 0.2 }, r#"{"type":"ramp","in":0.2,"out":0.2}"#),
    ];
    for (curve, text) in curves {
        assert_eq!(round_trip(&curve), text);
    }
    // the spec writes whole numbers without a fraction; they read the same
    let speeds: Curve = serde_json::from_str(r#"{"type":"speeds","start":0,"end":1.75}"#).expect("the spec's speeds");
    assert_eq!(speeds, Curve::Speeds { start: 0.0, end: 1.75 });
    let bezier: Curve = serde_json::from_str(r#"{"type":"bezier","x1":0.25,"y1":0.1,"x2":0.25,"y2":1}"#).expect("the spec's bezier");
    assert_eq!(bezier, Curve::Bezier { x1: 0.25, y1: 0.1, x2: 0.25, y2: 1.0 });
}

#[test]
fn t_m24_every_stored_type_round_trips() {
    let set = full_set(&hd());
    let text = round_trip(&set);
    let keys = [
        "plans",
        "walls",
        "useEditPoints",
        "suppressed",
        "candidates",
        "analysed",
        "parts",
        "fadeInFrames",
        "fadeOutFrames",
        "regions",
        "modifiedBy",
        "annotations",
        "boxTracks",
        "facing",
        "gone",
        "activeArea",
        "agentOverwrite",
        "sequence",
        "partAudio",
        "delivery",
        "reference",
        "startTimecode",
        "wallPairs",
        "deliveries",
        "profileDigest",
        "framingDigest",
        "outputDigest",
        "measuredLufs",
        "truePeakDbtp",
        "maxShortTermLufs",
        "lraLu",
        "minCorrelation",
        "conform",
        "runId",
        "nextPlanId",
        "nextPartId",
        "nextRegionId",
        "nextAnnotationId",
        "startingWindow",
        "captions",
        "maxLines",
        "keepLineBreaks",
        "minSize",
        "minClippedFrames",
        "fitBackground",
        "partApprovals",
        "attention",
        "overwritten",
        "nextPositionId",
        "nextRecordId",
        "captionAnchor",
        "subjects",
        "focus",
        "confidence",
        "risk",
        "flags",
        "proposed",
        "splitOf",
    ];
    for key in keys {
        assert!(has_key(&text, key), "{key} is missing from {text}");
    }
    for snake in ["use_edit_points", "box_tracks", "move_frames", "split_of", "modified_by", "next_plan_id", "r_in"] {
        assert!(!text.contains(snake), "{snake} is printed in snake_case");
    }
    // every enum value prints in camelCase
    for value in ["\"reanchored\"", "\"keepOut\"", "\"allOrNothing\"", "\"pictureChanged\"", "\"breakList\"", "\"catmullRom\"", "\"none\""] {
        assert!(text.contains(value), "{value} is missing from {text}");
    }
}

#[test]
fn t_m24_empty_optional_fields_are_omitted() {
    let r = hd();
    let set = VerticalSet { plans: vec![plan_of(vec![pos(&r, 1, 0, win(960.0, 540.0, 1080.0), Arrival::Cut)])], ..empty_set() };
    let text = round_trip(&set);
    let absent = [
        "parts",
        "regions",
        "annotations",
        "boxTracks",
        "activeArea",
        "agentOverwrite",
        "partAudio",
        "delivery",
        "reference",
        "deliveries",
        "stored",
        "suppressed",
        "candidates",
        "analysed",
        "startingWindow",
        "partApprovals",
        "attention",
        "overwritten",
        "track",
        "captionAnchor",
        "label",
        "color",
        "tags",
        "note",
        "subjects",
        "focus",
        "confidence",
        "risk",
        "flags",
        "proposed",
        "splitOf",
    ];
    for key in absent {
        assert!(!has_key(&text, key), "{key} is printed while empty: {text}");
    }
    for key in ["reviewed", "locked", "layout", "captions", "fitBackground", "nextRecordId", "nextAnnotationId", "edge", "minClippedFrames"] {
        assert!(has_key(&text, key), "{key} is missing from {text}");
    }
    // a key box that is not `gone` omits the flag, and the default part fades are omitted
    let key = KeyBox { at: tick(&r, 0), rect: [1.0, 2.0, 3.0, 4.0], facing: None, gone: false };
    assert_eq!(round_trip(&key), r#"{"at":0,"rect":[1.0,2.0,3.0,4.0]}"#);
    assert!(PartAudio::default().is_default() && !PartAudio { fade_in_frames: 3, fade_out_frames: 4 }.is_default());
    assert!(DeliveryInfo::default().is_empty() && PartMeta::default().is_empty());
}

#[test]
fn t_m24_ticks_are_integers_and_defaults_read_back() {
    let r = hd();
    let p = pos(&r, 31, 4805, win(812.4, 540.0, 1080.0), Arrival::Cut);
    let text = round_trip(&p);
    assert!(text.contains(&format!("\"at\":{}", tick(&r, 4805).0)), "{text}");
    // the fields a file may leave out read as their defaults
    let short = r#"{"id":1,"at":0,"window":{"cx":960.0,"cy":540.0,"h":1080.0},"arrival":{"kind":"cut"},"origin":"agent","modifiedBy":"agent"}"#;
    let read: Position = serde_json::from_str(short).expect("a short position reads");
    assert_eq!((read.layout, read.reviewed, read.locked, read.flags.len()), (Layout::Fill, false, false, 0));
    let captions: VerticalCaptions = serde_json::from_str(r#"{"maxLines":3}"#).expect("a short caption layout reads");
    assert_eq!(captions, VerticalCaptions { max_lines: 3, ..VerticalCaptions::default() });
    assert_eq!((captions.keep_line_breaks, captions.anchor, captions.edge), (true, CaptionAnchor::Bottom, CueEdge::Clip));
}

#[test]
fn t_m24_the_stored_plan_of_section_2_9_reads() {
    let text = r#"{"plans": [{"id": 1, "name": "9:16", "output": {"width": 1080, "height": 1920}, "filter": "catmullRom",
   "captions": {"maxLines": 2}, "fitBackground": "blur",
   "positions": [
     {"id": 31, "at": 48821875200000, "window": {"cx": 812.4, "cy": 540, "h": 1080}, "layout": "fill",
      "arrival": {"kind": "cut"}, "label": "Dana", "tags": ["speaker:Dana"], "note": "active speaker",
      "subjects": ["char:dana"], "origin": "agent", "modifiedBy": "agent"},
     {"id": 32, "at": 50142758400000, "window": {"cx": 948.2, "cy": 540, "h": 1080}, "layout": "fill",
      "arrival": {"kind": "move", "moveFrames": 40, "curve": {"type": "easeInOut"}},
      "label": "Dana", "tags": ["speaker:Dana"], "note": "follows her walk", "subjects": ["char:dana"],
      "origin": "agent", "modifiedBy": "human",
      "reviewed": true, "locked": true,
      "proposed": {"at": 50142758400000, "window": {"cx": 960, "cy": 540, "h": 1080}, "layout": "fill",
                   "arrival": {"kind": "move", "moveFrames": 50, "curve": {"type": "easeInOut"}}}}],
   "partApprovals": [],
   "nextPositionId": 33}],
 "walls": {"useEditPoints": true, "stored": [{"at": 48821875200000, "origin": "detected", "score": 0.87}],
           "candidates": [{"at": 49787136000000, "score": 0.19}],
           "analysed": [{"from": 0, "to": 328127708160000}]},
 "parts": [{"id": 7, "name": "P07", "from": 47785489920000, "to": 50650790400000}],
 "nextPlanId": 2, "nextPartId": 8, "nextRegionId": 1}"#;
    let set: VerticalSet = serde_json::from_str(text).expect("the stored plan of spec section 2.9 reads");
    let r = hd();
    let plan = &set.plans[0];
    assert_eq!((plan.positions.len(), plan.captions.max_lines, plan.next_position_id), (2, 2, 33));
    let (dana, walk) = (&plan.positions[0], &plan.positions[1]);
    assert_eq!((r.rate.frame_at(dana.at), r.rate.frame_at(walk.at)), (4805, 4935));
    assert_eq!(walk.arrival, mv(40, Curve::EaseInOut));
    assert!(walk.reviewed && walk.locked && walk.modified_by == Writer::Human && dana.origin == Origin::Agent);
    assert_eq!(walk.proposed.map(|p| p.arrival), Some(mv(50, Curve::EaseInOut)));
    assert_eq!((set.walls.stored[0].origin, set.parts[0].name.as_str()), (WallOrigin::Detected, "P07"));
    // frames 4703 to 4984, the end exclusive
    assert_eq!((r.rate.frame_at(set.parts[0].from), r.rate.frame_at(set.parts[0].to)), (4703, 4985));
    round_trip(&set);
}
