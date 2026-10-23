//! Synthetic test faces made from the bundled JetBrains Mono (no new font file): renamed, with a
//! character map of the characters a test needs (the space, digits and Latin letters on their own
//! glyphs, extra code points on the capitals in turn) and the weight, width class and italic flag
//! the test asks for. Every other table is copied as it is.

#![allow(dead_code)]

use std::path::PathBuf;

use filmcraft_text::{fonts, resolve};

/// Hebrew letters, U+05D0..U+05EA (final forms included).
pub const HEBREW: (u32, u32) = (0x5D0, 0x5EA);
/// Supplementary Private Use Area-A code points: no installed font covers them.
pub const PRIVATE: (u32, u32) = (0xF0100, 0xF011F);

/// One synthetic face.
pub struct Spec<'a> {
    pub family: &'a str,
    pub style: &'a str,
    pub weight: u16,
    /// OS/2 width class (5 = normal, 3 = condensed).
    pub width: u16,
    pub italic: bool,
    /// Code points mapped besides the space, digits and Latin letters (inclusive ranges).
    pub extra: &'a [(u32, u32)],
}

impl Spec<'_> {
    pub fn covers(&self, c: char) -> bool {
        c.is_ascii_alphanumeric() || c == ' ' || self.extra.iter().any(|(a, b)| (*a..=*b).contains(&(c as u32)))
    }
}

fn be16(b: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([b[o], b[o + 1]])
}

fn be32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// The face `spec` describes, as font file bytes.
pub fn font(spec: &Spec) -> Vec<u8> {
    let src = fonts::JETBRAINS_MONO_REGULAR;
    let face = fonts::face(resolve("JetBrains Mono", "Regular").face);
    assert_eq!(face.info.family, "JetBrains Mono", "fixture: the bundled source font");
    let gid = |c: char| face.glyph(c).unwrap_or_else(|| panic!("JetBrains Mono has a glyph for {c:?}"));
    let mut map: Vec<(u32, u32)> = (' '..='~').filter(|c| c.is_ascii_alphanumeric() || *c == ' ').map(|c| (c as u32, gid(c))).collect();
    let mut k = 0u8;
    for (a, b) in spec.extra {
        for c in *a..=*b {
            map.push((c, gid(char::from(b'A' + k % 26))));
            k = k.wrapping_add(1);
        }
    }
    map.sort_unstable();
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
    // name: family / style (also as the typographic family / subfamily), full and PostScript name
    let full = format!("{} {}", spec.family, spec.style);
    let ps = full.replace(' ', "");
    let names = [(1u16, spec.family.to_string()), (2, spec.style.to_string()), (4, full), (6, ps), (16, spec.family.to_string()), (17, spec.style.to_string())];
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
    // the source's other tables (OS/2 with the spec's weight, width class and italic flag), then the
    // font: table directory sorted by tag, 4-byte aligned data
    let mut tables: Vec<([u8; 4], Vec<u8>)> = vec![(*b"cmap", cmap), (*b"name", name)];
    for i in 0..be16(src, 4) as usize {
        let r = 12 + 16 * i;
        let tag: [u8; 4] = src[r..r + 4].try_into().unwrap();
        let (off, len) = (be32(src, r + 8) as usize, be32(src, r + 12) as usize);
        let mut t = src[off..off + len].to_vec();
        if &tag == b"OS/2" {
            t[4..6].copy_from_slice(&spec.weight.to_be_bytes());
            t[6..8].copy_from_slice(&spec.width.to_be_bytes());
            let fs: u16 = if spec.italic { 0x01 } else { 0x40 };
            t[62..64].copy_from_slice(&fs.to_be_bytes());
        }
        if !matches!(&tag, b"cmap" | b"name" | b"DSIG") {
            tables.push((tag, t));
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

/// Register the faces in memory (as a host registers a font file), in order; returns their ids.
pub fn register(specs: &[Spec]) -> Vec<fonts::FaceId> {
    let ids: Vec<_> = specs.iter().flat_map(|s| fonts::add_font_data(font(s))).collect();
    assert_eq!(ids.len(), specs.len(), "fixture: one face per font");
    filmcraft_text::layout::clear_cache();
    ids
}

/// Write the faces as font files into a fresh folder (`<name>-<n>.ttf`, in order) and add them as
/// a font source would find them on disk (origin "system", read on first use).
pub fn install(name: &str, specs: &[Spec]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("filmcraft-text-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (n, s) in specs.iter().enumerate() {
        std::fs::write(dir.join(format!("{name}-{n}.ttf")), font(s)).unwrap();
    }
    let added = fonts::add_source(&fonts::DirectorySource { dirs: vec![dir.clone()] });
    assert_eq!(added, specs.len(), "fixture: every face added");
    filmcraft_text::layout::clear_cache();
    dir
}

/// The face of a family and style name (exact).
pub fn face_named(family: &str, style: &str) -> std::sync::Arc<fonts::Face> {
    fonts::all_faces().into_iter().find(|f| f.info.family == family && f.info.style == style).unwrap_or_else(|| panic!("{family} {style} registered"))
}
