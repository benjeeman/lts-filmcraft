//! Hebrew and Arabic with the fonts installed on this machine; each test skips where the fonts it
//! needs are not installed.
//!
//! A SemiBold caption in Arial on Windows came out as missing-glyph boxes with condensed digits:
//! `resolve` picked Arial Narrow Bold (`ARIALNB.TTF`, no Hebrew; it sorts before `arialbd.ttf`),
//! and `fallback_for` found no loaded face with Hebrew. Hebrew or Arabic in Inter (no Hebrew, no
//! Arabic) found nothing either unless some other code had happened to read such a font.

use std::sync::Arc;

use filmcraft_text::bidi::is_rtl_char;
use filmcraft_text::fonts::{self, Face};
use filmcraft_text::{ParagraphStyle, TextStyle, layout, resolve};

const HEBREW: &str = "שלום, מה שלומך? 17 שקל";
const ARABIC: &str = "مرحبا بالعالم";

/// The installed faces of a family (the system font folders are scanned once).
fn installed(family: &str) -> Vec<Arc<Face>> {
    fonts::scan_system();
    fonts::all_faces().into_iter().filter(|f| f.info.origin == "system" && f.info.family.eq_ignore_ascii_case(family)).collect()
}

fn style(family: &str, style: &str) -> TextStyle {
    TextStyle { family: family.into(), style: style.into(), size: 60.0, ..Default::default() }
}

/// The character a glyph sets.
fn char_at(text: &str, cluster: usize) -> char {
    text[cluster..].chars().next().unwrap()
}

#[test]
fn hebrew_in_arial_semibold_is_set_in_a_regular_width_arial_face() {
    let arial = installed("Arial");
    if !arial.iter().any(|f| f.covers('ש')) {
        eprintln!("SKIPPED: no installed Arial face covers Hebrew");
        return;
    }
    let f = fonts::face(resolve("Arial", "SemiBold").face);
    assert_eq!(f.info.family, "Arial");
    assert_eq!(f.info.width, 5, "a regular-width face, not {} {}", f.info.style, f.info.width);
    assert!(HEBREW.chars().filter(|c| is_rtl_char(*c)).all(|c| f.has_char(c)), "Arial {} covers Hebrew", f.info.style);
    let l = layout(HEBREW, &style("Arial", "SemiBold"), &ParagraphStyle::default());
    assert!(!l.glyphs.is_empty());
    for g in &l.glyphs {
        let face = fonts::face(g.face);
        assert!(g.id != 0, "{:?}: a real glyph", char_at(HEBREW, g.cluster));
        assert_eq!((face.info.family.as_str(), face.info.width), ("Arial", 5), "{:?}: set in a regular-width Arial face", char_at(HEBREW, g.cluster));
    }
    // Arial Black has no Hebrew: its letters come from the Arial face nearest in weight that has
    if arial.iter().any(|f| f.info.style == "Black" && !f.covers('ש')) {
        let l = layout("שלום", &style("Arial", "Black"), &ParagraphStyle::default());
        for g in &l.glyphs {
            let face = fonts::face(g.face);
            assert!(g.id != 0 && face.info.family == "Arial" && !face.info.italic, "{} {}", face.info.family, face.info.style);
            assert_eq!(face.info.weight, 700, "Arial Bold, the nearest weight with Hebrew");
        }
    }
}

/// Hebrew and Arabic in Inter (which has neither) come from an installed face: Segoe UI where it is
/// installed, at the weight of the Inter face (Regular, SemiBold, Bold).
#[test]
fn hebrew_and_arabic_in_inter_come_from_an_installed_face_of_the_same_weight() {
    let segoe = installed("Segoe UI");
    let families = ["Segoe UI", "Arial", "Tahoma", "Times New Roman", "DejaVu Sans", "Noto Sans Hebrew", "Arial Hebrew"];
    let usable: Vec<Arc<Face>> = families.into_iter().flat_map(installed).collect();
    let mut checked = 0;
    for (text, sample) in [(HEBREW, 'ש'), (ARABIC, 'م')] {
        if !usable.iter().any(|f| f.covers(sample)) {
            eprintln!("SKIPPED {text}: no installed face covers it");
            continue;
        }
        for (name, weight) in [("Regular", 400), ("SemiBold", 600), ("Bold", 700)] {
            let l = layout(text, &style("Inter", name), &ParagraphStyle::default());
            for g in l.glyphs.iter().filter(|g| is_rtl_char(char_at(text, g.cluster))) {
                let (c, face) = (char_at(text, g.cluster), fonts::face(g.face));
                assert!(g.id != 0 && face.has_char(c), "{c:?} in Inter {name}: a real glyph");
                assert_eq!(face.info.origin, "system", "{c:?}: an installed face");
                if segoe.iter().any(|f| f.info.weight == weight && !f.info.italic && f.covers(sample)) {
                    assert_eq!((face.info.family.as_str(), face.info.weight), ("Segoe UI", weight), "{c:?} in Inter {name}: Segoe UI at weight {weight}");
                }
                checked += 1;
            }
        }
    }
    eprintln!("{checked} right-to-left glyphs checked");
}
