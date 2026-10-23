//! Which face sets a character (synthetic faces, so the outcome does not depend on the machine).
//!
//! A family's Narrow / Condensed faces lack scripts its regular-width faces have: Windows' Arial
//! Narrow Bold has no Hebrew while Arial Bold has. `resolve` broke a weight tie by file order, and
//! `ARIALNB.TTF` sorts before `arialbd.ttf`, so a SemiBold caption in Arial was set in Arial Narrow
//! Bold: condensed digits and a missing-glyph box for every Hebrew letter. `fallback_for` then
//! looked only at bundled, craft-fonts and already loaded faces, so it never found the family's own
//! Hebrew faces, nor any installed face not read yet. Now: a style the family lacks resolves to a
//! regular-width face, a character the chosen face lacks comes first from another face of its own
//! family, then from an installed face (read on demand, nearest in weight), and the choice depends
//! only on the font set, not on which files happen to be loaded.

mod common;

use common::{HEBREW, PRIVATE, Spec, face_named, install, register};
use filmcraft_text::fonts::{self, fallback_for};
use filmcraft_text::{ParagraphStyle, TextStyle, layout, resolve};

fn style(family: &str, style: &str) -> TextStyle {
    TextStyle { family: family.into(), style: style.into(), size: 40.0, ..Default::default() }
}

/// A SemiBold request in a family with Narrow Bold (no Hebrew) before Bold (Hebrew) at the same
/// distance in weight resolves to the regular-width Bold; a request that names Narrow keeps it.
#[test]
fn a_style_the_family_lacks_resolves_to_a_regular_width_face() {
    let fam = "LTS Width Test";
    register(&[
        Spec { family: fam, style: "Narrow Bold", weight: 700, width: 3, italic: false, extra: &[] },
        Spec { family: fam, style: "Bold", weight: 700, width: 5, italic: false, extra: &[HEBREW] },
        Spec { family: fam, style: "Regular", weight: 400, width: 5, italic: false, extra: &[HEBREW] },
    ]);
    assert_eq!(face_named(fam, "Narrow Bold").info.width, 3, "the OS/2 width class is read");
    let semibold = fonts::face(resolve(fam, "SemiBold").face);
    assert_eq!((semibold.info.style.as_str(), semibold.info.width), ("Bold", 5), "SemiBold: the regular-width Bold, not Narrow Bold");
    let narrow = fonts::face(resolve(fam, "Narrow SemiBold").face);
    assert_eq!(narrow.info.style, "Narrow Bold", "a style that asks for Narrow keeps the narrow face");
    assert_eq!(fonts::face(resolve(fam, "Narrow Bold").face).info.style, "Narrow Bold", "an exact style is kept");
    // and Hebrew in that family at that weight is drawn, in the family
    let l = layout("שלום עולם", &style(fam, "SemiBold"), &ParagraphStyle::default());
    assert_eq!(l.glyphs.len(), 9);
    for g in &l.glyphs {
        assert!(g.id != 0, "a real glyph: {g:?}");
        assert_eq!(fonts::face(g.face).info.family, fam);
    }
}

/// Hebrew in a face of the family that lacks it (Black, as Arial Black) comes from the family's own
/// Hebrew face nearest in weight, not from another family and not as missing-glyph boxes, even
/// though nothing has read that face yet.
#[test]
fn a_character_the_face_lacks_comes_from_its_own_family_first() {
    let fam = "LTS Family Test";
    register(&[
        Spec { family: fam, style: "Black", weight: 900, width: 5, italic: false, extra: &[] },
        Spec { family: fam, style: "Regular", weight: 400, width: 5, italic: false, extra: &[HEBREW] },
        Spec { family: fam, style: "Bold", weight: 700, width: 5, italic: false, extra: &[HEBREW] },
    ]);
    // another registered family covers Hebrew too: the requested family still wins
    register(&[Spec { family: "LTS Family Other", style: "Black", weight: 900, width: 5, italic: false, extra: &[HEBREW] }]);
    let black = resolve(fam, "Black").face;
    assert_eq!(fonts::face(black).info.style, "Black");
    let l = layout("שלום", &style(fam, "Black"), &ParagraphStyle::default());
    for g in &l.glyphs {
        let f = fonts::face(g.face);
        assert!(g.id != 0, "a real glyph: {g:?}");
        assert_eq!((f.info.family.as_str(), f.info.style.as_str()), (fam, "Bold"), "the family's Hebrew face nearest in weight");
    }
    // Latin stays in the requested face
    let latin = layout("Abc", &style(fam, "Black"), &ParagraphStyle::default());
    assert!(latin.glyphs.iter().all(|g| g.face == black && g.id != 0));
}

/// A character no bundled or registered face covers comes from an installed face that has not been
/// read yet (a font file on disk, as the system scan finds them).
#[test]
fn a_character_no_loaded_face_has_comes_from_an_installed_face() {
    let dir = install("disk", &[Spec { family: "LTS Disk Test", style: "Regular", weight: 400, width: 5, italic: false, extra: &[PRIVATE] }]);
    let inter = resolve("Inter", "Regular").face;
    let c = char::from_u32(PRIVATE.0).unwrap();
    assert!(!fonts::face(inter).has_char(c));
    let f = fonts::face(fallback_for(c, inter));
    assert_eq!((f.info.family.as_str(), f.info.origin), ("LTS Disk Test", "system"), "the face on disk serves it");
    let text: String = (PRIVATE.0..PRIVATE.0 + 4).filter_map(char::from_u32).collect();
    let l = layout(&text, &TextStyle { size: 40.0, ..Default::default() }, &ParagraphStyle::default());
    assert!(l.glyphs.len() == 4 && l.glyphs.iter().all(|g| g.id != 0), "{:?}", l.glyphs);
    let _ = std::fs::remove_dir_all(dir);
}

/// Among installed faces that cover a character, the one nearest in weight to the requested face
/// serves it, whichever of them happens to be loaded already; the same font set gives the same
/// choice every time.
#[test]
fn the_installed_fallback_nearest_in_weight_wins_whatever_is_loaded() {
    let fam = "LTS Weight Test";
    let extra = [(PRIVATE.0 + 0x20, PRIVATE.0 + 0x2F)];
    let dir = install(
        "weight",
        &[
            Spec { family: fam, style: "Regular", weight: 400, width: 5, italic: false, extra: &extra },
            Spec { family: fam, style: "Bold", weight: 700, width: 5, italic: false, extra: &extra },
        ],
    );
    let c = char::from_u32(extra[0].0).unwrap();
    // the Bold file is read first
    assert!(face_named(fam, "Bold").font().is_some());
    let regular = fonts::face(fallback_for(c, resolve("Inter", "Regular").face));
    assert_eq!((regular.info.family.as_str(), regular.info.style.as_str()), (fam, "Regular"), "Inter Regular: the Regular face");
    let bold = fonts::face(fallback_for(c, resolve("Inter", "Bold").face));
    assert_eq!(bold.info.style, "Bold", "Inter Bold: the Bold face");
    let semibold = fonts::face(fallback_for(c, resolve("Inter", "SemiBold").face));
    assert_eq!(semibold.info.style, "Bold", "SemiBold (600) is nearer Bold (700) than Regular (400)");
    for _ in 0..3 {
        assert_eq!(fallback_for(c, resolve("Inter", "Regular").face), regular.id, "stable");
    }
    let _ = std::fs::remove_dir_all(dir);
}
