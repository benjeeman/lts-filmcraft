//! Model-visible images from the worker (native-worker-architecture.md §6.5, INV-48 refined).
//!
//! The lab's trusted parent canonicalizes every image by decoding and re-encoding it in the
//! browser. A native worker has no browser, so the work splits: the worker encodes a fresh PNG or
//! JPEG from the engine's RGBA and writes no metadata ([`encode_png`], [`encode_jpeg`]); its
//! structure is then checked without decoding ([`check_png`], [`check_jpeg`], which the host
//! repeats); and the broker hashes and binds the bytes it forwards.
//!
//! A PNG may hold only `IHDR`, `PLTE`, `IDAT` and `IEND`, plus an optional `sRGB`; text chunks
//! (`tEXt`, `zTXt`, `iTXt`), `eXIf` and `iCCP` are refused. A JPEG may hold only SOI, APP0 (JFIF),
//! DQT, SOF0 or SOF2, DHT, DRI, SOS and EOI; APP1 (EXIF, XMP), APP2 (ICC) and comments are refused.
//! The header's width and height must be the ones the caller expects.

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder};

/// The PNG signature (ISO/IEC 15948 §5.2).
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// The chunks a model-visible PNG may hold.
const PNG_CHUNKS: [&[u8; 4]; 5] = [b"IHDR", b"PLTE", b"IDAT", b"IEND", b"sRGB"];

/// The longest chunk a PNG may declare (ISO/IEC 15948 §5.3: 2³¹ − 1 bytes).
const PNG_MAX_CHUNK: usize = 0x7FFF_FFFF;

/// The longest side a baseline JPEG header can state.
const JPEG_MAX_SIDE: u32 = 65_535;

/// The bytes of `width` × `height` pixels of `channels` bytes each, when that is a size.
fn pixel_bytes(width: u32, height: u32, channels: usize) -> Option<usize> {
    let pixels = usize::try_from(width).ok()?.checked_mul(usize::try_from(height).ok()?)?;
    pixels.checked_mul(channels).filter(|&bytes| bytes > 0)
}

/// A fresh PNG of `width` × `height` RGBA pixels (`rgba`, rows top to bottom), with no metadata
/// chunk.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    if pixel_bytes(width, height, 4) != Some(rgba.len()) {
        return Err(format!("{} bytes are not {width}×{height} RGBA pixels", rgba.len()));
    }
    let mut png = Vec::new();
    PngEncoder::new(&mut png).write_image(rgba, width, height, ExtendedColorType::Rgba8).map_err(|e| format!("PNG encoding failed: {e}"))?;
    check_png(&png, width, height)?;
    Ok(png)
}

/// A fresh baseline JPEG of `width` × `height` RGBA pixels at `quality` (1-100), alpha dropped,
/// with no metadata segment.
pub fn encode_jpeg(width: u32, height: u32, rgba: &[u8], quality: u8) -> Result<Vec<u8>, String> {
    if pixel_bytes(width, height, 4) != Some(rgba.len()) {
        return Err(format!("{} bytes are not {width}×{height} RGBA pixels", rgba.len()));
    }
    if width > JPEG_MAX_SIDE || height > JPEG_MAX_SIDE {
        return Err(format!("a JPEG side is at most {JPEG_MAX_SIDE} pixels, not {width}×{height}"));
    }
    let rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|pixel| [pixel[0], pixel[1], pixel[2]]).collect();
    let mut jpeg = Vec::new();
    let encoder = JpegEncoder::new_with_quality(&mut jpeg, quality.clamp(1, 100));
    encoder.write_image(&rgb, width, height, ExtendedColorType::Rgb8).map_err(|e| format!("JPEG encoding failed: {e}"))?;
    check_jpeg(&jpeg, width, height)?;
    Ok(jpeg)
}

/// A big-endian `u32` at `at`.
fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    let word = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([word[0], word[1], word[2], word[3]]))
}

/// A big-endian `u16` at `at`.
fn be16(bytes: &[u8], at: usize) -> Option<u16> {
    let word = bytes.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([word[0], word[1]]))
}

/// CRC-32 as PNG computes it (ISO 3309: reflected, polynomial 0xEDB88320).
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// A chunk type as text, for a refusal.
fn chunk_name(kind: &[u8]) -> String {
    kind.iter().map(|&b| if b.is_ascii_alphanumeric() { char::from(b) } else { '?' }).collect()
}

/// Whether `bytes` is a PNG of `width` × `height` that holds only the chunks a model-visible image
/// may hold; the error names what is refused. Every chunk's CRC is checked, `IHDR` comes first,
/// at least one `IDAT` follows it and `IEND` ends the file.
pub fn check_png(bytes: &[u8], width: u32, height: u32) -> Result<(), String> {
    let body = bytes.strip_prefix(PNG_SIGNATURE).ok_or("not a PNG: the signature is missing")?;
    let mut at = 0usize;
    let mut first = true;
    let mut data = false;
    while at < body.len() {
        let length = be32(body, at).ok_or("a PNG chunk header is cut short")? as usize;
        if length > PNG_MAX_CHUNK {
            return Err(format!("a PNG chunk declares {length} bytes"));
        }
        let kind = body.get(at + 4..at + 8).ok_or("a PNG chunk header is cut short")?;
        let end = at + 8 + length;
        let payload = body.get(at + 8..end).ok_or_else(|| format!("PNG chunk {} is cut short", chunk_name(kind)))?;
        let stored = be32(body, end).ok_or_else(|| format!("PNG chunk {} has no CRC", chunk_name(kind)))?;
        let covered = body.get(at + 4..end).ok_or("a PNG chunk is cut short")?;
        if crc32(covered) != stored {
            return Err(format!("PNG chunk {} fails its CRC", chunk_name(kind)));
        }
        if !PNG_CHUNKS.iter().any(|allowed| allowed.as_slice() == kind) {
            return Err(format!("PNG chunk {} is not allowed in a model-visible image", chunk_name(kind)));
        }
        if first != (kind == b"IHDR") {
            return Err("IHDR must be the first PNG chunk, and the only one".to_string());
        }
        if first {
            if length != 13 {
                return Err(format!("IHDR holds {length} bytes, not 13"));
            }
            let (w, h) = (be32(payload, 0).unwrap_or(0), be32(payload, 4).unwrap_or(0));
            if (w, h) != (width, height) {
                return Err(format!("the PNG is {w}×{h}, not {width}×{height}"));
            }
            first = false;
        }
        data |= kind == b"IDAT";
        at = end + 4;
        if kind == b"IEND" {
            if length != 0 || at != body.len() {
                return Err("IEND must be empty and end the PNG".to_string());
            }
            return if data { Ok(()) } else { Err("the PNG has no image data".to_string()) };
        }
    }
    Err("the PNG ends before IEND".to_string())
}

/// A JPEG marker as text, for a refusal.
fn marker_name(marker: u8) -> String {
    match marker {
        0xE0..=0xEF => format!("APP{}", marker - 0xE0),
        0xFE => "COM".to_string(),
        0xC0..=0xCF => format!("SOF{} (0xFF{marker:02X})", marker - 0xC0),
        _ => format!("0xFF{marker:02X}"),
    }
}

/// The entropy-coded data of a scan from `at`: the offset of the marker that ends it (restart
/// markers and stuffed bytes belong to the scan).
fn scan_end(bytes: &[u8], mut at: usize) -> Option<usize> {
    loop {
        if *bytes.get(at)? != 0xFF {
            at += 1;
            continue;
        }
        match *bytes.get(at + 1)? {
            0x00 | 0xD0..=0xD7 => at += 2,
            0xFF => at += 1,
            _ => return Some(at),
        }
    }
}

/// Whether an APP0 payload is a bare JFIF header: the identifier, the version, the density and no
/// thumbnail (a thumbnail is a second picture).
fn bare_jfif(payload: &[u8]) -> bool {
    payload.len() == 14 && payload.starts_with(b"JFIF\0") && payload[12] == 0 && payload[13] == 0
}

/// Whether `bytes` is a JPEG of `width` × `height` that holds only the segments a model-visible
/// image may hold; the error names what is refused. APP0 must be a bare JFIF header (no
/// thumbnail), one SOF0 or SOF2 states the size, and EOI ends the file.
pub fn check_jpeg(bytes: &[u8], width: u32, height: u32) -> Result<(), String> {
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return Err("not a JPEG: SOI is missing".to_string());
    }
    let mut at = 2usize;
    let mut size = None;
    let mut scanned = false;
    loop {
        if bytes.get(at) != Some(&0xFF) {
            return Err("the JPEG ends before EOI, or a marker is missing".to_string());
        }
        let marker = *bytes.get(at + 1).ok_or("the JPEG ends before EOI")?;
        if marker == 0xD9 {
            if at + 2 != bytes.len() {
                return Err("bytes follow the JPEG's EOI".to_string());
            }
            return match (size, scanned) {
                (Some(_), true) => Ok(()),
                _ => Err("the JPEG has no frame header or no scan".to_string()),
            };
        }
        if !matches!(marker, 0xE0 | 0xDB | 0xC0 | 0xC2 | 0xC4 | 0xDD | 0xDA) {
            return Err(format!("JPEG segment {} is not allowed in a model-visible image", marker_name(marker)));
        }
        let length = usize::from(be16(bytes, at + 2).ok_or("a JPEG segment header is cut short")?);
        let payload = bytes.get(at + 4..at + 2 + length).ok_or_else(|| format!("JPEG segment {} is cut short", marker_name(marker)))?;
        match marker {
            0xE0 if !bare_jfif(payload) => {
                return Err("APP0 must be a bare JFIF header without a thumbnail".to_string());
            }
            0xC0 | 0xC2 => {
                if size.is_some() {
                    return Err("the JPEG has more than one frame header".to_string());
                }
                let (h, w) = (be16(payload, 1).ok_or("SOF is cut short")?, be16(payload, 3).ok_or("SOF is cut short")?);
                if (u32::from(w), u32::from(h)) != (width, height) {
                    return Err(format!("the JPEG is {w}×{h}, not {width}×{height}"));
                }
                size = Some((w, h));
            }
            _ => {}
        }
        at += 2 + length;
        if marker == 0xDA {
            if size.is_none() {
                return Err("a JPEG scan comes before its frame header".to_string());
            }
            scanned = true;
            at = scan_end(bytes, at).ok_or("the JPEG ends inside a scan, before EOI")?;
        }
    }
}
