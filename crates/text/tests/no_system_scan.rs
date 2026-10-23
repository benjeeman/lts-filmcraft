//! With the system font scan off (`set_system_scan(false)`, as the LTS worker runs: OC-33 E1),
//! characters the bundled faces lack are served by the faces the host registered, and only by them:
//! no system font folder is read to find one, so the result depends on the registered font set
//! alone. A face registered before anything read it serves them too (the fallback looked only at
//! faces whose bytes something had already loaded).

mod common;

use common::{HEBREW, Spec, register};
use filmcraft_text::fonts;
use filmcraft_text::layout::layout_uncached;
use filmcraft_text::{ParagraphStyle, TextStyle};

fn system_faces() -> usize {
    fonts::all_faces().iter().filter(|f| f.info.origin == "system").count()
}

/// One test, its steps in order: the switch and the font database are process-wide.
#[test]
fn with_the_scan_off_only_registered_faces_serve_missing_characters() {
    fonts::set_system_scan(false);
    let st = TextStyle { size: 40.0, ..Default::default() };
    let text = "שלום";
    let bare = layout_uncached(text, &st, &ParagraphStyle::default());
    assert!(bare.glyphs.iter().all(|g| g.id == 0), "no bundled face covers Hebrew: missing glyphs, {:?}", bare.glyphs);
    assert!(!fonts::system_scanned() && system_faces() == 0, "no system font folder was read");

    register(&[Spec { family: "LTS Registered Hebrew", style: "Regular", weight: 400, width: 5, italic: false, extra: &[HEBREW] }]);
    let l = layout_uncached(text, &st, &ParagraphStyle::default());
    for g in &l.glyphs {
        assert!(g.id != 0, "a real glyph: {g:?}");
        assert_eq!(fonts::face(g.face).info.family, "LTS Registered Hebrew", "the registered face serves it");
    }
    assert!(!fonts::system_scanned() && system_faces() == 0, "still no system font folder read");
    let again = layout_uncached(text, &st, &ParagraphStyle::default());
    assert_eq!(again, l, "the same font set gives the same layout");
}
