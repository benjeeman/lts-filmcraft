//! Caption burn-in draws with the track style's font (`CaptionStyle::font`), Hebrew included, as
//! Premiere's caption properties do (OC-33 E2). `burn.rs` set every caption in Inter SemiBold
//! whatever the style named, so a Hebrew caption came out in whichever fallback face covered
//! Hebrew, or as missing-glyph boxes.
//!
//! The drawn face is told by its advances: the overlay's width follows the text's width in the
//! requested face. The Hebrew faces are made here from the bundled JetBrains Mono and Noto Serif
//! (no new font file): renamed, with a character map that puts the space and the 27 Hebrew letters
//! on glyphs of the source font, so each covers Hebrew with advances of its own (fixed in the
//! monospaced one, varying in the serif one).

use std::sync::OnceLock;

use filmcraft_captions::burn::{Overlay, render_caption};
use filmcraft_project::{Caption, CaptionStyle, ClipId};
use filmcraft_text::{TextStyle, fonts, measure, resolve};
use filmcraft_time::Tick;

const W: usize = 1920;
const H: usize = 1080;
/// The caption size at 1080 lines (CaptionStyle's default, 54 px).
const PX: f32 = 54.0;
const HEBREW_MONO: &str = "LTS Test Hebrew Mono";
const HEBREW_SERIF: &str = "LTS Test Hebrew Serif";
const HEBREW: &str = "שלום עולם";

fn caption(text: &str) -> Caption {
    Caption { id: ClipId(1), start: Tick(0), duration: Tick(100), text: text.into(), speaker: None, cue_id: None, settings: String::new() }
}

fn burn(text: &str, font: &str) -> Overlay {
    let style = CaptionStyle { font: font.into(), ..Default::default() };
    assert_eq!(style.size, PX, "fixture: the default caption size");
    render_caption(&caption(text), &style, W, H).unwrap_or_else(|| panic!("{font}: a caption overlay"))
}

/// The text's width set in `family` at the caption weight (SemiBold, synthesised where the family
/// has only a Regular).
fn width_in(family: &str, text: &str) -> f32 {
    measure(text, &TextStyle { family: family.into(), style: "SemiBold".into(), size: PX, ..Default::default() })
}

fn be16(b: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([b[o], b[o + 1]])
}

fn be32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// The bundled font `src` (family `src_family`) renamed to `family`, its character map replaced by
/// one (format 12) that maps the space to the space glyph and U+05D0..U+05EA to the glyphs of the
/// capitals A..Z (in turn). Every other table is copied as it is.
fn hebrew_test_font(src: &[u8], src_family: &str, family: &str) -> Vec<u8> {
    let face = fonts::face(resolve(src_family, "Regular").face);
    assert_eq!(face.info.family, src_family, "fixture: the bundled source font");
    let gid = |c: char| face.glyph(c).unwrap_or_else(|| panic!("{src_family} has a glyph for {c:?}"));
    let mut map: Vec<(u32, u32)> = vec![(0x20, gid(' '))];
    for (k, c) in (0x5D0u32..=0x5EA).enumerate() {
        map.push((c, gid(char::from(b'A' + (k % 26) as u8))));
    }
    // cmap: one Windows Unicode full-repertoire (3, 10) subtable, format 12, one group per character
    let mut cmap = Vec::new();
    for v in [0u16, 1, 3, 10] {
        cmap.extend(v.to_be_bytes());
    }
    cmap.extend(12u32.to_be_bytes());
    cmap.extend(12u16.to_be_bytes());
    cmap.extend(0u16.to_be_bytes());
    cmap.extend((16 + 12 * map.len() as u32).to_be_bytes());
    cmap.extend(0u32.to_be_bytes());
    cmap.extend((map.len() as u32).to_be_bytes());
    for (c, g) in &map {
        for v in [*c, *c, *g] {
            cmap.extend(v.to_be_bytes());
        }
    }
    // name: family, subfamily, full name and PostScript name, Windows Unicode English
    let names = [(1u16, family.to_string()), (2, "Regular".into()), (4, format!("{family} Regular")), (6, format!("{}-Regular", family.replace(' ', "")))];
    let strings: Vec<Vec<u8>> = names.iter().map(|(_, s)| s.encode_utf16().flat_map(u16::to_be_bytes).collect()).collect();
    let mut name = Vec::new();
    for v in [0u16, names.len() as u16, 6 + 12 * names.len() as u16] {
        name.extend(v.to_be_bytes());
    }
    let mut at = 0u16;
    for ((id, _), s) in names.iter().zip(&strings) {
        for v in [3u16, 1, 0x409, *id, s.len() as u16, at] {
            name.extend(v.to_be_bytes());
        }
        at += s.len() as u16;
    }
    for s in &strings {
        name.extend(s);
    }
    // the source's other tables, then the font: table directory sorted by tag, 4-byte aligned data
    let mut tables: Vec<([u8; 4], Vec<u8>)> = vec![(*b"cmap", cmap), (*b"name", name)];
    for i in 0..be16(src, 4) as usize {
        let r = 12 + 16 * i;
        let tag: [u8; 4] = src[r..r + 4].try_into().unwrap();
        let (off, len) = (be32(src, r + 8) as usize, be32(src, r + 12) as usize);
        if !matches!(&tag, b"cmap" | b"name" | b"DSIG") {
            tables.push((tag, src[off..off + len].to_vec()));
        }
    }
    tables.sort_by_key(|t| t.0);
    let n = tables.len() as u16;
    let selector = (15 - n.leading_zeros()) as u16;
    let range = 16u16 << selector;
    let mut out = src[0..4].to_vec();
    for v in [n, range, selector, n * 16 - range] {
        out.extend(v.to_be_bytes());
    }
    let mut data = Vec::new();
    let mut offset = 12 + 16 * tables.len();
    for (tag, t) in &tables {
        out.extend(tag);
        out.extend(0u32.to_be_bytes());
        out.extend((offset as u32).to_be_bytes());
        out.extend((t.len() as u32).to_be_bytes());
        data.extend(t);
        let padded = (t.len() + 3) & !3;
        data.resize(data.len() + padded - t.len(), 0);
        offset += padded;
    }
    out.extend(data);
    out
}

/// The two Hebrew test faces, registered once for this test binary (the font database is
/// process-wide). The monospaced one is registered first and loaded, so it is the face a fallback
/// for Hebrew would find.
fn hebrew_fonts() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let sources = [(fonts::JETBRAINS_MONO_REGULAR, "JetBrains Mono", HEBREW_MONO), (fonts::NOTO_SERIF_REGULAR, "Noto Serif", HEBREW_SERIF)];
        for (src, src_family, family) in sources {
            let ids = fonts::add_font_data(hebrew_test_font(src, src_family, family));
            assert_eq!(ids.len(), 1, "{family}: one face registered");
            let f = fonts::face(ids[0]);
            assert_eq!(f.info.family, family);
            assert!(HEBREW.chars().all(|c| f.has_char(c)), "{family} covers {HEBREW}");
        }
    });
}

#[test]
fn the_hebrew_test_faces_are_usable() {
    hebrew_fonts();
    for family in [HEBREW_MONO, HEBREW_SERIF] {
        let r = resolve(family, "SemiBold");
        assert!(!r.missing, "{family} resolves");
        assert_eq!(fonts::face(r.face).info.family, family);
    }
    // monospaced: the nine characters as wide as nine Latin ones in the source face
    let (hebrew, latin) = (width_in(HEBREW_MONO, HEBREW), width_in("JetBrains Mono", "ABCDE FGH"));
    assert!((hebrew - latin).abs() < 0.5, "{hebrew} vs {latin}");
    let gap = width_in(HEBREW_SERIF, HEBREW) - width_in(HEBREW_MONO, HEBREW);
    assert!(gap.abs() > 8.0, "fixture: the two faces set the line at different widths ({gap})");
}

#[test]
fn a_latin_caption_is_drawn_in_the_requested_font() {
    let text = "Hello world";
    let inter = burn(text, "Inter");
    let mono = burn(text, "JetBrains Mono");
    let want = width_in("JetBrains Mono", text) - width_in("Inter", text);
    assert!(want > 20.0, "fixture: JetBrains Mono sets the line wider ({want})");
    let got = mono.w as f32 - inter.w as f32;
    assert!((got - want).abs() <= 2.0, "the overlay is as much wider as the text in JetBrains Mono: {got} px, want {want}");
    assert!(mono.px.chunks(4).any(|p| p[3] > 0.99 && p[0] > 0.9), "white text drawn");
}

#[test]
fn a_hebrew_caption_is_drawn_in_the_requested_font_not_a_fallback() {
    hebrew_fonts();
    let mono = burn(HEBREW, HEBREW_MONO);
    let serif = burn(HEBREW, HEBREW_SERIF);
    let want = width_in(HEBREW_SERIF, HEBREW) - width_in(HEBREW_MONO, HEBREW);
    let got = serif.w as f32 - mono.w as f32;
    assert!((got - want).abs() <= 2.0, "each caption is set in its own face: {got} px apart, want {want}");
    // and against a Latin caption in the bundled monospaced face of the same nine-character width
    let latin = burn("ABCDE FGH", "JetBrains Mono");
    assert!((mono.w as f32 - latin.w as f32).abs() <= 2.0, "nine monospaced characters: {} vs {}", mono.w, latin.w);
    for (o, family) in [(&mono, HEBREW_MONO), (&serif, HEBREW_SERIF)] {
        assert!(o.px.chunks(4).any(|p| p[3] > 0.99 && p[0] > 0.9), "{family}: white text drawn");
    }
}

#[test]
fn the_default_and_an_unknown_font_draw_as_before() {
    let text = "Hello world";
    let default = render_caption(&caption(text), &CaptionStyle::default(), W, H).expect("an overlay");
    assert_eq!(burn(text, "Inter"), default, "Inter is the default");
    assert_eq!(burn(text, ""), default, "no font named: Inter");
    assert_eq!(burn(text, "No Such Caption Font"), default, "a font that is not installed falls back to Inter");
}
