//! Caption burn-in sets each line in its track's direction (audit issue H3). A track in a
//! right-to-left language (Hebrew, Arabic, Persian, Urdu, Yiddish) lays every line out right to
//! left, so a Hebrew line that starts with a Latin word reads from the right, the Latin word at the
//! right end; on any other track a line takes its direction from its first strong character, as
//! before. `burn.rs` laid every line out with `ParagraphStyle::default()`: the first strong
//! character always decided, and such a Hebrew line came out left to right.
//!
//! The direction is told by where the ink sits: three wide Latin capitals and two Hebrew letters
//! ten spaces apart (light, wide). Left to right the capitals are at the left, so the ink sits left
//! of the overlay's centre; right to left they are at the right end.

use filmcraft_captions::burn::{Overlay, rtl_language, track_overlay};
use filmcraft_project::{Caption, CaptionFormat, CaptionStyle, CaptionTrack, ClipId, TrackId};
use filmcraft_time::Tick;

const LATIN_FIRST: &str = "WWW א          ב";
const HEBREW_FIRST: &str = "א          ב WWW";

fn track(language: &str, text: &str) -> CaptionTrack {
    let mut t = CaptionTrack::new(TrackId(1), "S".into(), CaptionFormat::Subtitle);
    t.language = language.into();
    t.style = CaptionStyle { background: false, ..Default::default() };
    t.captions.push(Caption { id: ClipId(1), start: Tick(0), duration: Tick(100), text: text.into(), speaker: None, cue_id: None, settings: String::new() });
    t
}

/// The coverage-weighted mean x of the overlay's pixels, as a fraction of its width.
fn ink_centre(o: &Overlay) -> f64 {
    let (mut sum, mut mass) = (0.0f64, 0.0f64);
    for (i, p) in o.px.chunks(4).enumerate() {
        sum += p[3] as f64 * (i % o.w) as f64;
        mass += p[3] as f64;
    }
    assert!(mass > 100.0, "text is drawn");
    sum / mass / o.w as f64
}

fn centre(language: &str, text: &str) -> f64 {
    ink_centre(&track_overlay(&track(language, text), Tick(5), 1920, 1080).expect("a caption overlay"))
}

#[test]
fn right_to_left_track_languages() {
    for l in ["he", "he-IL", "HE", "iw", "ar", "ar-EG", "fa", "fa-IR", "ur", "yi", "heb", "ara"] {
        assert!(rtl_language(l), "{l} is written right to left");
    }
    for l in ["en", "", "fr-CA", "ru", "ja", "hu", "eng", "zh-Hant"] {
        assert!(!rtl_language(l), "{l} is not");
    }
}

#[test]
fn a_latin_first_hebrew_line_reads_right_to_left_on_a_right_to_left_track() {
    for language in ["he", "he-IL", "ar", "fa", "ur", "yi"] {
        let c = centre(language, LATIN_FIRST);
        assert!(c > 0.55, "{language}: the Latin word at the right end (ink centre {c:.3})");
    }
    for language in ["en", ""] {
        let c = centre(language, LATIN_FIRST);
        assert!(c < 0.45, "{language:?}: from the first strong character, left to right (ink centre {c:.3})");
    }
}

#[test]
fn a_hebrew_first_line_reads_right_to_left_on_any_track() {
    for language in ["he", "en", ""] {
        let c = centre(language, HEBREW_FIRST);
        assert!(c < 0.45, "{language:?}: right to left from its first letter, the Latin word at the left end (ink centre {c:.3})");
    }
}
