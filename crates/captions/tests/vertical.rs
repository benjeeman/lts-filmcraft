//! The vertical subtitle layout and the part sidecar (Vertical Edit, lane L3b; spec §6.4, §6.5; §11 T-R9's layout
//! clauses, T-R10, T-R11 and T-R12). Synthetic cues only. The direction checks use the ink oracle of `burn_rtl.rs`:
//! three wide Latin capitals and two Hebrew letters ten spaces apart, so the side the capitals land on shows the
//! direction whatever face draws the Hebrew.

use filmcraft_captions::burn::{
    BurnedCue, CaptionBox, CaptionOverflow, Layout, Overlay, PartRange, caption_overflow, cue_box, cue_overlay, cues_at, cues_in_range, layout_tracks,
    sequence_overlays_in,
};
use filmcraft_captions::{RangeOptions, RtlMarks, cue_in_range, document_from_track, document_from_track_range};
use filmcraft_project::vertical::{CaptionAnchor, CueEdge};
use filmcraft_project::{Caption, CaptionFormat, CaptionStyle, CaptionTrack, ClipId, Project, Sequence, SequenceSettings, TrackId};
use filmcraft_time::{FrameRate, Tick, TimeRange};

const RATE: FrameRate = FrameRate::FPS_25;
const RLM: char = '\u{200F}';
const LATIN_FIRST: &str = "WWW א          ב";

fn cue(id: u64, from: i64, to: i64, text: &str, settings: &str) -> Caption {
    let (start, end) = (RATE.tick_of(from), RATE.tick_of(to));
    Caption { id: ClipId(id), start, duration: end - start, text: text.into(), speaker: None, cue_id: None, settings: settings.into() }
}

fn track(id: u64, language: &str, cues: Vec<Caption>) -> CaptionTrack {
    let mut t = CaptionTrack::new(TrackId(id), format!("Subtitles {id}"), CaptionFormat::Subtitle);
    t.language = language.into();
    t.captions = cues;
    t
}

/// [`track`] without the background box, so only the text draws ink.
fn plain_track(id: u64, language: &str, cues: Vec<Caption>) -> CaptionTrack {
    let mut t = track(id, language, cues);
    t.style = CaptionStyle { background: false, ..CaptionStyle::default() };
    t
}

/// The default 9:16 layout of a 1080×1920 output: the Union safe area, 54 px down to 40 px, two lines, at the bottom.
fn layout() -> Layout {
    Layout {
        area: [120.0, 288.0, 768.0, 960.0],
        size_px: Some(54.0),
        min_px: Some(40.0),
        max_lines: 2,
        keep_breaks: true,
        anchor: CaptionAnchor::Bottom,
        track: None,
        part: None,
    }
}

/// The `n`-th cue of a track, burned over its whole span.
fn burned(t: &CaptionTrack, n: usize) -> BurnedCue<'_> {
    let cue = &t.captions[n];
    BurnedCue { track: t, cue, from: cue.start, to: cue.start + cue.duration }
}

/// The `n`-th cue laid out in a 1080×1920 output.
fn boxed(t: &CaptionTrack, n: usize, l: &Layout) -> CaptionBox {
    cue_box(&burned(t, n), 1080, 1920, l).unwrap_or_else(|| panic!("cue {n} is laid out"))
}

/// A 1920×1080 25p sequence with the caption tracks `tracks` (the first is the top track).
fn sequence(tracks: Vec<CaptionTrack>) -> Sequence {
    let mut p = Project::new("captions");
    let id = p.new_sequence("s", SequenceSettings { width: 1920, height: 1080, frame_rate: RATE, ..Default::default() }, 1, 0, None);
    let mut seq = p.sequence(id).unwrap().clone();
    seq.caption_tracks = tracks;
    seq
}

/// The frames `[from, to)`.
fn range(from: i64, to: i64) -> TimeRange {
    TimeRange::new(RATE.tick_of(from), RATE.tick_of(to) - RATE.tick_of(from))
}

fn opts(edge: CueEdge, min_clipped_frames: u32, rtl_marks: RtlMarks) -> RangeOptions {
    RangeOptions { edge, min_clipped_frames, rtl_marks, rate: RATE }
}

/// Where the ink of an overlay sits within a line's box: the coverage-weighted mean x of the overlay's pixels whose
/// centre lies in the box's rows, as a fraction of the box's width from its left edge.
fn ink_centre(o: &Overlay, line: [f32; 4]) -> f64 {
    let [bx, by, bw, bh] = line.map(f64::from);
    let (mut sum, mut mass) = (0.0f64, 0.0f64);
    for (n, p) in o.px.chunks_exact(4).enumerate() {
        let (x, y) = (f64::from(o.x) + (n % o.w) as f64 + 0.5, f64::from(o.y) + (n / o.w) as f64 + 0.5);
        if (by..by + bh).contains(&y) {
            sum += f64::from(p[3]) * x;
            mass += f64::from(p[3]);
        }
    }
    assert!(mass > 50.0, "the line {line:?} draws ink");
    (sum / mass - bx) / bw
}

/// T-R9 (spec §6.4 "Layout"): the subtitler's line breaks are kept, each source line on its own line; without
/// `keep_breaks` the lines are joined with a space.
#[test]
fn t_r9_source_line_breaks_are_kept() {
    let t = track(1, "en", vec![cue(1, 0, 50, "Short one\nAnd two", "")]);
    let b = boxed(&t, 0, &layout());
    assert_eq!(b.text, ["Short one", "And two"]);
    assert_eq!((b.size_px, b.overflow, b.lines.len()), (54.0, false, 2));
    assert_eq!((b.track, b.cue, b.anchor), (TrackId(1), ClipId(1), CaptionAnchor::Bottom));
    let joined = boxed(&t, 0, &Layout { keep_breaks: false, ..layout() });
    assert_eq!(joined.text, ["Short one And two"]);
    assert_eq!(joined.lines.len(), 1);
}

/// T-R9: each line wraps so that its box (its text and the padding) fits the area's width, and while the cue needs more
/// than `max_lines` lines the size shrinks in 2 % steps (× 0.98), stopping at the first step that fits: one step
/// bigger it would not fit. Two source lines that each wrap at 54 px fit on one line each once shrunk.
#[test]
fn t_r9_lines_fit_the_area_and_the_size_shrinks_in_2_percent_steps() {
    let text = "Each window keeps its subtitles\ninside the safe area of a phone";
    let t = track(1, "en", vec![cue(1, 0, 50, text, "")]);
    let l = layout();
    let at_54 = boxed(&t, 0, &Layout { min_px: Some(54.0), ..l.clone() });
    assert!(at_54.overflow && at_54.text.len() > 2, "at 54 px the lines wrap: {at_54:?}");
    let b = boxed(&t, 0, &l);
    assert!(!b.overflow, "{b:?}");
    assert_eq!(b.text, ["Each window keeps its subtitles", "inside the safe area of a phone"]);
    assert!(b.size_px < 54.0 && b.size_px >= 40.0, "the size shrank, not below the minimum: {}", b.size_px);
    let k = ((b.size_px / 54.0).ln() / 0.98f32.ln()).round();
    assert!(k >= 1.0 && (54.0 * 0.98f32.powi(k as i32) - b.size_px).abs() <= 0.01, "{} px is not 54 px × 0.98^k", b.size_px);
    let previous = b.size_px / 0.98;
    let bigger = boxed(&t, 0, &Layout { size_px: Some(previous), min_px: Some(previous), ..l.clone() });
    assert!(bigger.overflow && bigger.text.len() > 2, "one step bigger ({previous} px) the cue needs more than two lines: {bigger:?}");
    let [ax, _, aw, _] = l.area;
    for (n, line) in b.lines.iter().enumerate() {
        let [x, _, w, _] = *line;
        assert!(x >= ax - 0.01 && x + w <= ax + aw + 0.01, "line {n}, {line:?}, is wider than the area");
    }
}

/// T-R9: shrinking stops at the minimum size. A cue that needs more lines than `max_lines` even there is laid out at
/// the minimum with every line, and reported `caption_overflow {cue, lines, frame}` at its first frame; with room for
/// its lines it keeps the full size.
#[test]
fn t_r9_shrinking_stops_at_the_minimum_and_overflow_is_reported() {
    let t = track(1, "en", vec![cue(7, 12, 60, "One\nTwo\nThree", ""), cue(8, 70, 90, "Fits", "")]);
    let l = layout();
    let b = boxed(&t, 0, &l);
    assert_eq!((b.size_px, b.overflow, b.text.len(), b.lines.len()), (40.0, true, 3, 3));
    let roomy = boxed(&t, 0, &Layout { max_lines: 3, ..l.clone() });
    assert_eq!((roomy.size_px, roomy.overflow, roomy.text.len()), (54.0, false, 3));
    let seq = sequence(vec![t]);
    let warning = CaptionOverflow { cue: ClipId(7), lines: 3, frame: 12 };
    assert_eq!(caption_overflow(&seq, range(0, 100), 1080, 1920, &l), [warning]);
    assert_eq!(caption_overflow(&seq, range(30, 100), 1080, 1920, &l), [warning], "the cue's first frame, wherever the range starts");
    assert!(caption_overflow(&seq, range(60, 100), 1080, 1920, &l).is_empty(), "no overflowing cue in the range");
    assert!(caption_overflow(&seq, range(0, 100), 1080, 1920, &Layout { max_lines: 3, ..l }).is_empty());
}

/// T-R9: the block sits at the area's bottom or top by the layout's anchor; a WebVTT `line:` setting places it relative
/// to the area, kept inside it; the line boxes stay inside the area's width.
#[test]
fn t_r9_anchor_and_webvtt_line_are_relative_to_the_area() {
    let cues = vec![cue(1, 0, 50, "Anchored", ""), cue(2, 50, 100, "Half way", "line:50%"), cue(3, 100, 150, "Low", "line:100%")];
    let t = track(1, "en", cues);
    let l = layout();
    let [ax, ay, aw, ah] = l.area;
    let bottom = boxed(&t, 0, &l);
    assert!((bottom.rect[1] + bottom.rect[3] - (ay + ah)).abs() <= 1.0, "bottom: {bottom:?}");
    let top = boxed(&t, 0, &Layout { anchor: CaptionAnchor::Top, ..l.clone() });
    assert!((top.rect[1] - ay).abs() <= 1.0, "top: {top:?}");
    let half = boxed(&t, 1, &l);
    assert!((half.rect[1] - (ay + ah * 0.5)).abs() <= 1.0, "line:50%: {half:?}");
    let low = boxed(&t, 2, &l);
    assert!((low.rect[1] + low.rect[3] - (ay + ah)).abs() <= 1.0, "line:100% keeps the block inside the area: {low:?}");
    for b in [&bottom, &top, &half, &low] {
        assert!(b.rect[0] >= ax && b.rect[0] + b.rect[2] <= ax + aw, "{b:?} is not inside the area's width");
        assert_eq!(b.lines.len(), 1);
    }
}

/// Which tracks burn: every enabled track, bottom track first (as the 16:9 burn-in), or the layout's track only,
/// whether or not its output is on; and the cues they burn at a tick and over a range.
#[test]
fn layout_tracks_and_the_cues_they_burn() {
    let mut off = track(3, "en", vec![cue(31, 0, 50, "Off", "")]);
    off.enabled = false;
    let seq = sequence(vec![track(1, "en", vec![cue(11, 0, 50, "Top track", "")]), track(2, "he", vec![cue(21, 10, 50, "שלום", "")]), off]);
    let ids = |l: &Layout| layout_tracks(&seq, l).iter().map(|t| t.id).collect::<Vec<_>>();
    assert_eq!(ids(&layout()), [TrackId(2), TrackId(1)], "the enabled tracks, bottom first");
    assert_eq!(ids(&Layout { track: Some(3), ..layout() }), [TrackId(3)], "the chosen track, although its output is off");
    assert!(ids(&Layout { track: Some(9), ..layout() }).is_empty(), "no such track");
    let at = |frame: i64| cues_at(&seq, &layout(), RATE.tick_of(frame)).iter().map(|c| c.cue.id).collect::<Vec<_>>();
    assert_eq!(at(5), [ClipId(11)]);
    assert_eq!(at(20), [ClipId(21), ClipId(11)]);
    assert!(at(50).is_empty());
    let spans: Vec<(ClipId, Tick, Tick)> = cues_in_range(&seq, &layout(), range(40, 45)).iter().map(|c| (c.cue.id, c.from, c.to)).collect();
    assert_eq!(spans, [(ClipId(21), RATE.tick_of(10), RATE.tick_of(50)), (ClipId(11), Tick::ZERO, RATE.tick_of(50))]);
    assert_eq!(sequence_overlays_in(&seq, RATE.tick_of(20), 1080, 1920, &layout()).len(), 2, "one overlay per burned cue");
}

/// T-R9 and T-R11 (spec §6.4 "Direction"): a two-line cue on a Hebrew track lays each line out right to left, so a
/// Latin word that starts a line sits at its right end; on an English track the same lines read from their first
/// strong character, left to right.
#[test]
fn t_r9_a_two_line_hebrew_cue_lays_out_right_to_left() {
    for (language, rtl) in [("he", true), ("en", false)] {
        let t = plain_track(1, language, vec![cue(1, 0, 50, &format!("{LATIN_FIRST}\n{LATIN_FIRST}"), "")]);
        let b = boxed(&t, 0, &layout());
        assert_eq!(b.lines.len(), 2, "{language}: two lines");
        let o = cue_overlay(&burned(&t, 0), 1080, 1920, &layout()).expect("the cue is drawn");
        for line in &b.lines {
            let c = ink_centre(&o, *line);
            if rtl {
                assert!(c > 0.55, "{language}: the Latin word at the right end of {line:?} (ink centre {c:.3})");
            } else {
                assert!(c < 0.45, "{language}: left to right in {line:?} (ink centre {c:.3})");
            }
        }
    }
}

/// T-R11 (spec §6.4 "Direction"): WebVTT `align:start` and `align:end` resolve against the line's direction. On a
/// Hebrew track `start` puts a Latin-first Hebrew line at the area's right edge and `end` at its left; on an English
/// track that line is left to right and the reverse holds, while a Hebrew-first line takes `start` to the right.
/// `align:left` and `align:right` mean what they say; no setting is the style's centre.
#[test]
fn t_r11_align_start_and_end_follow_the_direction() {
    let edges = |language: &str, text: &str, settings: &str| {
        let t = plain_track(1, language, vec![cue(1, 0, 50, text, settings)]);
        let [x, _, w, _] = boxed(&t, 0, &layout()).lines[0];
        (x, x + w)
    };
    let [ax, _, aw, _] = layout().area;
    let left = |e: (f32, f32)| (e.0 - ax).abs() <= 0.5;
    let right = |e: (f32, f32)| (e.1 - (ax + aw)).abs() <= 0.5;
    assert!(right(edges("he", LATIN_FIRST, "align:start")), "he, start: the right edge");
    assert!(left(edges("he", LATIN_FIRST, "align:end")), "he, end: the left edge");
    assert!(left(edges("en", LATIN_FIRST, "align:start")), "en, a Latin-first line, start: the left edge");
    assert!(right(edges("en", LATIN_FIRST, "align:end")), "en, a Latin-first line, end: the right edge");
    assert!(right(edges("en", "שלום עולם", "align:start")), "en, a Hebrew-first line, start: the right edge");
    assert!(left(edges("he", LATIN_FIRST, "align:left")) && right(edges("he", LATIN_FIRST, "align:right")));
    let (x0, x1) = edges("he", LATIN_FIRST, "");
    assert!(((x0 + x1) * 0.5 - (ax + aw * 0.5)).abs() <= 0.5, "no setting: centred ({x0}–{x1})");
    let t = plain_track(1, "he", vec![cue(1, 0, 50, LATIN_FIRST, "align:start")]);
    let b = boxed(&t, 0, &layout());
    let o = cue_overlay(&burned(&t, 0), 1080, 1920, &layout()).expect("the cue is drawn");
    let ink = ink_centre(&o, [ax, b.lines[0][1], aw, b.lines[0][3]]);
    assert!(ink > 0.5, "the ink of a Hebrew line aligned to start lies in the right half of the area (centre {ink:.3})");
    assert!(ink_centre(&o, b.lines[0]) > 0.55, "and reads from the right");
}

/// T-R10 (spec §6.5, the part-sidecar regression of 41 fact 1): the range document keeps exactly the cues that overlap
/// the range, clipped to it, with times from its start, and none after its end (which the old `document_from_track`
/// keeps); the track's language and WebVTT blocks; each cue's id, voice and settings.
#[test]
fn t_r10_the_range_document_keeps_exactly_the_cues_in_the_range() {
    let cues = vec![
        cue(1, 0, 10, "before", ""),
        cue(2, 10, 30, "into the start", ""),
        cue(3, 40, 60, "inside", "align:start"),
        cue(4, 70, 72, "short", ""),
        cue(5, 90, 110, "over the end", ""),
        cue(6, 100, 120, "from the end", ""),
        cue(7, 130, 150, "after", ""),
    ];
    let mut t = track(1, "he", cues);
    t.vtt_blocks = vec!["STYLE\n::cue { color: yellow }".into()];
    t.captions[2].speaker = Some("Dana".into());
    t.captions[2].cue_id = Some("c3".into());
    let doc = document_from_track_range(&t, range(20, 100), &opts(CueEdge::Clip, 0, RtlMarks::None));
    let got: Vec<(Tick, Tick, &str)> = doc.cues.iter().map(|c| (c.start, c.end, c.text.as_str())).collect();
    let f = |n: i64| RATE.tick_of(n);
    assert_eq!(got, [(f(0), f(10), "into the start"), (f(20), f(40), "inside"), (f(50), f(52), "short"), (f(70), f(80), "over the end")]);
    assert_eq!(doc.language.as_deref(), Some("he"));
    assert_eq!(doc.blocks, t.vtt_blocks);
    let inside = &doc.cues[1];
    assert_eq!((inside.speaker.as_deref(), inside.id.as_deref(), inside.settings.as_str()), (Some("Dana"), Some("c3"), "align:start"));
    let old = document_from_track(&t, f(20));
    assert!(old.cues.iter().any(|c| c.text == "after"), "the old document keeps the cues after the range (41 fact 1)");
    let mut untagged = t.clone();
    untagged.language = String::new();
    assert_eq!(document_from_track_range(&untagged, range(20, 100), &opts(CueEdge::Clip, 0, RtlMarks::None)).language, None);
}

/// T-R10: a cue that straddles an edge is clipped to the range or dropped by the edge rule, and a clipped remainder
/// shorter than `min_clipped_frames` is dropped; a short cue inside the range stays.
#[test]
fn t_r10_straddling_cues_are_clipped_or_dropped() {
    let t = track(1, "en", vec![cue(1, 10, 30, "a", ""), cue(2, 40, 60, "b", ""), cue(3, 70, 72, "c", ""), cue(4, 90, 110, "d", "")]);
    let texts = |o: RangeOptions| document_from_track_range(&t, range(20, 100), &o).cues.iter().map(|c| c.text.clone()).collect::<Vec<_>>();
    assert_eq!(texts(opts(CueEdge::Clip, 0, RtlMarks::None)), ["a", "b", "c", "d"]);
    assert_eq!(texts(opts(CueEdge::Drop, 0, RtlMarks::None)), ["b", "c"]);
    assert_eq!(texts(opts(CueEdge::Clip, 10, RtlMarks::None)), ["a", "b", "c", "d"], "remainders of 10 frames stay at 10");
    assert_eq!(texts(opts(CueEdge::Clip, 11, RtlMarks::None)), ["b", "c"], "and go at 11; the 2-frame cue inside stays");
}

/// The one edge rule of the burn-in and the sidecar (`cue_in_range`): kept whole inside, clipped or dropped across an
/// edge, the remainder counted in frames shown (on frame starts), touching edges, empty input and hostile ticks.
#[test]
fn cue_in_range_is_the_one_edge_rule() {
    let f = |n: i64| RATE.tick_of(n);
    let keep = |a: Tick, b: Tick, edge: CueEdge, min: u32| cue_in_range(a, b, range(20, 100), edge, min, RATE);
    assert_eq!(keep(f(40), f(60), CueEdge::Clip, 0), Some((f(40), f(60))));
    assert_eq!(keep(f(40), f(42), CueEdge::Drop, 99), Some((f(40), f(42))), "inside: kept whole, however short");
    assert_eq!(keep(f(10), f(30), CueEdge::Clip, 0), Some((f(20), f(30))));
    assert_eq!(keep(f(10), f(30), CueEdge::Drop, 0), None);
    assert_eq!(keep(f(90), f(110), CueEdge::Clip, 10), Some((f(90), f(100))));
    assert_eq!(keep(f(90), f(110), CueEdge::Clip, 11), None);
    assert_eq!(keep(f(10), f(110), CueEdge::Clip, 0), Some((f(20), f(100))), "over both edges");
    assert_eq!(keep(f(10), f(20), CueEdge::Clip, 0), None, "ends on the start");
    assert_eq!(keep(f(100), f(110), CueEdge::Clip, 0), None, "starts on the end");
    assert_eq!(keep(f(50), f(50), CueEdge::Clip, 0), None, "empty cue");
    assert_eq!(keep(f(60), f(50), CueEdge::Clip, 0), None, "a cue that ends before it starts");
    assert_eq!(cue_in_range(f(40), f(60), TimeRange::new(f(20), Tick::ZERO), CueEdge::Clip, 0, RATE), None, "empty range");
    let late = f(98) + Tick(1);
    assert_eq!(keep(late, f(105), CueEdge::Clip, 2), None, "from inside frame 98 to the edge only frame 99 shows");
    assert_eq!(keep(late, f(105), CueEdge::Clip, 1), Some((late, f(100))));
    let far = TimeRange::new(Tick(i64::MAX - 10), Tick(i64::MAX));
    for (a, b) in [(Tick(i64::MIN), Tick(i64::MAX)), (Tick(i64::MAX - 1), Tick(i64::MAX)), (Tick(i64::MIN), Tick(i64::MIN + 1))] {
        for edge in [CueEdge::Clip, CueEdge::Drop] {
            let _ = cue_in_range(a, b, far, edge, u32::MAX, RATE);
            let _ = cue_in_range(a, b, TimeRange::new(Tick(i64::MIN), Tick(i64::MAX)), edge, 3, FrameRate { num: 0, den: 0 });
        }
    }
}

/// T-R10: right-to-left marks per `rtl_marks`. `Auto` marks the lines of a right-to-left track that do not begin with a
/// strong right-to-left character (a line that begins with a direction mark keeps its own), `Rlm` every line, `None`
/// none; an English track keeps its text with `Auto`.
#[test]
fn t_r10_rtl_marks() {
    let lines = ["שלום עולם!", "iPhone חדש בחנות.", "10 שקלים בלבד", "<i>שלום</i>", "Hello world.", "\u{200F}כבר מסומן", "\u{200E}LTR on purpose"];
    let text = lines.join("\n");
    let marked = |language: &str, marks: RtlMarks| {
        let t = track(1, language, vec![cue(1, 0, 50, &text, "")]);
        let doc = document_from_track_range(&t, range(0, 50), &opts(CueEdge::Clip, 0, marks));
        doc.cues[0].text.split('\n').map(str::to_string).collect::<Vec<_>>()
    };
    let with = |l: &str| format!("{RLM}{l}");
    let same = |l: &str| l.to_string();
    let auto = [same(lines[0]), with(lines[1]), with(lines[2]), with(lines[3]), with(lines[4]), same(lines[5]), same(lines[6])];
    assert_eq!(marked("he", RtlMarks::Auto), auto);
    let every = [with(lines[0]), with(lines[1]), with(lines[2]), with(lines[3]), with(lines[4]), same(lines[5]), with(lines[6])];
    assert_eq!(marked("he", RtlMarks::Rlm), every);
    assert_eq!(marked("he", RtlMarks::None), lines);
    assert_eq!(marked("en", RtlMarks::Auto), lines);
    assert_eq!(marked("en", RtlMarks::Rlm), every);
}

/// T-R12 (spec §6.4 "At part edges"): in a part export the burn-in follows the edge rule and agrees with the sidecar:
/// with `clip` a straddling cue is burned only on the part's frames, with `drop` not at all, and a remainder shorter
/// than `min_clipped_frames` not at all. On every frame a cue is burned exactly when the part's sidecar has one.
#[test]
fn t_r12_the_burn_in_follows_the_edge_rule_like_the_sidecar() {
    let cues = vec![
        cue(1, 10, 40, "ends on the in-edge", ""),
        cue(2, 30, 50, "over the in-edge", ""),
        cue(3, 60, 70, "inside", ""),
        cue(4, 95, 110, "over the out-edge", ""),
        cue(5, 100, 120, "after", ""),
    ];
    let t = track(1, "en", cues);
    let seq = sequence(vec![t.clone()]);
    let part = range(40, 100);
    let clipped: &[(i64, i64)] = &[(40, 50), (60, 70), (95, 100)];
    let cases = [(CueEdge::Clip, 0, clipped), (CueEdge::Drop, 0, &[(60, 70)][..]), (CueEdge::Clip, 6, &[(40, 50), (60, 70)][..])];
    for (edge, min, spans) in cases {
        let l = Layout { part: Some(PartRange { range: part, edge, min_clipped_frames: min }), ..layout() };
        let doc = document_from_track_range(&t, part, &opts(edge, min, RtlMarks::Auto));
        for frame in 0..130 {
            let at = RATE.tick_of(frame);
            let burned = !cues_at(&seq, &l, at).is_empty();
            let expected = spans.iter().any(|&(a, b)| (a..b).contains(&frame));
            let rel = at - part.start;
            let in_sidecar = (40..100).contains(&frame) && doc.cues.iter().any(|c| (c.start..c.end).contains(&rel));
            assert_eq!(burned, expected, "{edge:?}, min {min}: frame {frame}");
            assert_eq!(burned, in_sidecar, "{edge:?}, min {min}: frame {frame}: the burn-in and the sidecar disagree");
        }
        let overlays = sequence_overlays_in(&seq, RATE.tick_of(45), 1080, 1920, &l);
        assert_eq!(overlays.len(), usize::from(edge == CueEdge::Clip), "{edge:?}, min {min}: the cue over the in-edge at frame 45");
    }
}

/// The old document of a track (`document_from_track`) is unchanged for its callers: cues ending after the offset,
/// starts clamped to zero, times from the offset, no language.
#[test]
fn document_from_track_is_unchanged() {
    let t = track(1, "he", vec![cue(1, 0, 10, "gone", ""), cue(2, 10, 30, "clamped", ""), cue(3, 40, 60, "kept", "")]);
    let doc = document_from_track(&t, RATE.tick_of(20));
    let got: Vec<(Tick, Tick, &str)> = doc.cues.iter().map(|c| (c.start, c.end, c.text.as_str())).collect();
    assert_eq!(got, [(Tick::ZERO, RATE.tick_of(10), "clamped"), (RATE.tick_of(20), RATE.tick_of(40), "kept")]);
    assert_eq!(doc.language, None);
}

/// Never crash (FilmCraft AGENTS.md §0): sizes and areas that are not finite, zero, negative or huge, no lines allowed,
/// empty and markup-only cues, odd WebVTT settings, frames that are not frame sizes and hostile ticks give no layout or
/// a bounded one, never a panic.
#[test]
fn hostile_layouts_never_panic() {
    let cues = vec![cue(1, 0, 50, "שלום\n\n<i></i>\nWWW", "line:-50% align:start"), cue(2, 50, 60, "", ""), cue(3, 60, 70, "<b></b>", "line:1e309% align:")];
    let t = track(1, "he", cues);
    let layouts = [
        Layout { size_px: Some(f32::NAN), ..layout() },
        Layout { size_px: Some(-3.0), min_px: Some(f32::INFINITY), ..layout() },
        Layout { size_px: Some(1e30), ..layout() },
        Layout { min_px: Some(f32::NAN), ..layout() },
        Layout { area: [f32::NAN, 0.0, 100.0, 100.0], ..layout() },
        Layout { area: [0.0, 0.0, 0.0, 0.0], ..layout() },
        Layout { area: [-1e30, -1e30, 1e30, 1e30], ..layout() },
        Layout { area: [0.0, 0.0, 1.0, 1.0], max_lines: 0, ..layout() },
        Layout { max_lines: 255, keep_breaks: false, ..layout() },
    ];
    let seq = sequence(vec![t.clone()]);
    for l in &layouts {
        for n in 0..3 {
            let c = burned(&t, n);
            if let Some(b) = cue_box(&c, 1080, 1920, l) {
                assert!(b.size_px.is_finite() && b.size_px > 0.0 && b.size_px <= 1920.0, "{l:?}: {b:?}");
            }
            if let Some(o) = cue_overlay(&c, 1080, 1920, l) {
                assert!(o.w * o.h <= 1080 * 1920 && o.px.len() == o.w * o.h * 4, "{l:?}: a {}×{} overlay", o.w, o.h);
            }
        }
        for at in [Tick(i64::MIN), Tick::ZERO, Tick(i64::MAX)] {
            let _ = sequence_overlays_in(&seq, at, 1080, 1920, l);
        }
        let _ = caption_overflow(&seq, TimeRange::new(Tick(i64::MIN), Tick(i64::MAX)), 1080, 1920, l);
    }
    for (w, h) in [(0, 0), (1, 1), (usize::MAX, 2), (2, usize::MAX)] {
        let _ = cue_overlay(&burned(&t, 0), w, h, &layout());
        let _ = cue_box(&burned(&t, 0), w, h, &layout());
    }
}
