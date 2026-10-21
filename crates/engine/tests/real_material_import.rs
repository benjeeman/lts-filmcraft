//! Newsroom material in the engine (batch BF-2 of the LTS editor bug fixes, OC-18; master plan v3
//! W7; `logs/lts-media/real-material-gaps.md`, the Film table): what a session shows, mixes and
//! reports when it imports a damaged recording, a 5.1 source, an off-air transport stream capture or
//! a cut download. The files are made here, in an in-memory host: exported by the engine itself and
//! then damaged or cut, or written by hand (a WAV, a transport stream). Nothing is read from disk
//! and nothing is downloaded.
//!
//! | Test | Gap | What it pins |
//! |---|---|---|
//! | `a_damaged_intra_frame_shows_the_last_good_picture_and_is_counted` | damaged streams | concealment, and the count in `metadata.get` |
//! | `a_damaged_inter_frame_does_not_freeze_the_rest_of_its_gop` | damaged streams (the GOP cache stuck) | decoding moves past the bad sample |
//! | `a_damaged_audio_packet_is_concealed_and_counted` | damaged streams (corrupt AAC) | silence in its place, and the count |
//! | `a_51_source_on_a_stereo_track_is_downmixed` | AAC 5.1 and every 5.1 source | ITU-R BS.775 on a stereo track, not two channels picked |
//! | `a_transport_stream_capture_imports_in_the_background` | TS captures, long files | the caller's thread never scans the whole capture |
//! | `a_cut_faststart_download_imports_clamped_with_a_warning` | cut fast-start download | a shorter clip and a warning in the Events log |
//!
//! `metadata.get` of a media item reports `damage: {videoSamplesConcealed, audioPacketsConcealed}`:
//! the samples of the item's media that failed to decode and were concealed so far, each counted
//! once. The source-level half of the batch is `crates/codecs/tests/real_material.rs`.

use std::collections::BTreeMap;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

use filmcraft_engine::panels::Level;
use filmcraft_engine::project::ItemId;
use filmcraft_engine::{Services, Session};
use filmcraft_media::{ByteReader, SharedReader};
use filmcraft_time::FrameRate;
use serde_json::{Value, json};

// ------------------------------------------------------------------------------------ the host

/// A host whose files live in memory: what a test puts there and what an export writes (exports are
/// encoded in memory and handed to `write_file`). It counts the bytes read on the thread that made
/// it, the caller's thread.
struct MemHost {
    files: Mutex<BTreeMap<String, Arc<[u8]>>>,
    caller: ThreadId,
    read_on_caller: Arc<AtomicU64>,
}

impl MemHost {
    fn new() -> Arc<MemHost> {
        Arc::new(MemHost { files: Mutex::default(), caller: std::thread::current().id(), read_on_caller: Arc::default() })
    }

    fn put(&self, path: &str, bytes: Vec<u8>) {
        self.files.lock().unwrap().insert(path.to_string(), bytes.into());
    }

    fn get(&self, path: &str) -> io::Result<Arc<[u8]>> {
        let files = self.files.lock().unwrap();
        files.get(path).cloned().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{path}: not in the test host")))
    }

    fn count(&self, bytes: usize) {
        if std::thread::current().id() == self.caller {
            self.read_on_caller.fetch_add(bytes as u64, Ordering::SeqCst);
        }
    }

    fn read_on_caller(&self) -> u64 {
        self.read_on_caller.load(Ordering::SeqCst)
    }
}

/// A reader of one in-memory file that counts what it reads on the caller's thread.
struct MemFile {
    bytes: Arc<[u8]>,
    caller: ThreadId,
    read_on_caller: Arc<AtomicU64>,
}

impl ByteReader for MemFile {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let range = self.bytes.get(start..start.saturating_add(buf.len())).ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))?;
        buf.copy_from_slice(range);
        if std::thread::current().id() == self.caller {
            self.read_on_caller.fetch_add(buf.len() as u64, Ordering::SeqCst);
        }
        Ok(())
    }
}

impl Services for MemHost {
    fn read_file(&self, path: &str) -> io::Result<Vec<u8>> {
        let bytes = self.get(path)?;
        self.count(bytes.len());
        Ok(bytes.to_vec())
    }

    fn write_file(&self, path: &str, data: &[u8]) -> io::Result<()> {
        self.put(path, data.to_vec());
        Ok(())
    }

    fn file_size(&self, path: &str) -> io::Result<u64> {
        Ok(self.get(path)?.len() as u64)
    }

    fn read_range(&self, path: &str, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let bytes = self.get(path)?;
        let start = usize::try_from(offset).unwrap_or(usize::MAX).min(bytes.len());
        let range = &bytes[start..start.saturating_add(len).min(bytes.len())];
        self.count(range.len());
        Ok(range.to_vec())
    }

    fn reader(&self, path: &str) -> Option<io::Result<SharedReader>> {
        let file = |bytes| Arc::new(MemFile { bytes, caller: self.caller, read_on_caller: self.read_on_caller.clone() }) as SharedReader;
        Some(self.get(path).map(file))
    }

    fn export_in_memory(&self) -> bool {
        true
    }
}

// ------------------------------------------------------------------------------------ sessions

/// A session over `host` with a new 64×32 sequence at 25 fps (stereo mix, stereo tracks).
fn session(host: &Arc<MemHost>) -> Session {
    let mut s = Session::new(host.clone());
    s.execute("file.newSequence", json!({"name": "BF-2", "width": 64, "height": 32, "fps": 25})).expect("a sequence opens");
    s
}

/// Animated demo footage (the Plasma scene, ten seconds of picture and sound); answers the item.
fn demo_footage(s: &mut Session) -> u64 {
    let imported = s.execute("file.importDemoFootage", json!({"scene": "Plasma"})).expect("the demo footage imports");
    imported["items"][0].as_u64().unwrap_or_else(|| panic!("one demo item: {imported}"))
}

/// Imports `path` from the host; answers the item.
fn import(s: &mut Session, path: &str) -> u64 {
    let imported = s.execute("file.import", json!({"paths": [path]})).unwrap_or_else(|e| panic!("{path} imports: {e}"));
    imported["items"][0].as_u64().unwrap_or_else(|| panic!("one item from {path}: {imported}"))
}

/// Places `item` at the start of `track` (a video track takes the sound to A1) for `frames` frames.
fn place(s: &mut Session, item: u64, track: &str, frames: i64) {
    let duration = FrameRate::FPS_25.tick_of(frames).0;
    s.execute("timeline.place", json!({"item": item, "track": track, "time": 0, "duration": duration})).expect("the clip is placed");
}

/// Exports the active sequence to `path` in the host with `params` (a format and its settings),
/// waiting for it; answers the file.
fn export(s: &mut Session, host: &MemHost, path: &str, params: Value) -> Vec<u8> {
    let mut params = params;
    params["path"] = json!(path);
    params["wait"] = json!(true);
    s.execute("file.exportMedia", params).unwrap_or_else(|e| panic!("the export to {path} runs: {e}"));
    host.get(path).expect("the export is in the host").to_vec()
}

/// Frames 0 to `count` (excluded) of the active sequence at full size, 8 bits over black.
fn render(s: &Session, count: i64) -> Vec<Vec<u8>> {
    let rate = s.sequence_rate();
    (0..count).map(|k| s.try_render_program_at(1.0, rate.tick_of(k)).expect("the sequence renders").over_black_rgba8()).collect()
}

/// Imports `path` into a fresh session over `host`, places it on V1 for `count` frames and renders
/// them; answers the frames, the session and the item.
fn imported(host: &Arc<MemHost>, path: &str, count: i64) -> (Vec<Vec<u8>>, Session, u64) {
    let mut s = session(host);
    let item = import(&mut s, path);
    place(&mut s, item, "V1", count);
    (render(&s, count), s, item)
}

/// What a frame shows with nothing on the timeline.
fn empty_frame(host: &Arc<MemHost>) -> Vec<u8> {
    render(&session(host), 1).remove(0)
}

/// The `damage` record `metadata.get` reports for `item`.
fn damage(s: &mut Session, item: u64) -> Value {
    let metadata = s.execute("metadata.get", json!({"item": item})).expect("the metadata reads");
    metadata["damage"].clone()
}

/// `frames` stereo samples of the active sequence's mix from sample `start` (48 kHz).
fn mix(s: &Session, start: i64, frames: usize) -> Vec<Vec<f32>> {
    let provider = s.media.provider(s.project.clone(), s.services.clone());
    let sequence = s.active_sequence().expect("an active sequence");
    filmcraft_engine::render::audio::mix_sequence(&s.project, sequence, start, frames, &provider).channels
}

// ------------------------------------------------------------------------------------ MP4 tables

/// One track of an MP4 or MOV as its sample tables give it: its handler (`vide`, `soun`) and each
/// sample's file offset and size, in decode order.
struct TrackTable {
    handler: [u8; 4],
    samples: Vec<(usize, usize)>,
}

fn u32_at(data: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(data[at..at + 4].try_into().unwrap())
}

/// The boxes directly inside `data`, as (type, payload).
fn boxes(data: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let (mut out, mut at) = (Vec::new(), 0usize);
    while at + 8 <= data.len() {
        let kind: [u8; 4] = data[at + 4..at + 8].try_into().unwrap();
        let (header, size) = match u32_at(data, at) {
            1 => (16, u64::from_be_bytes(data[at + 8..at + 16].try_into().unwrap()) as usize),
            0 => (8, data.len() - at),
            size => (8, size as usize),
        };
        out.push((kind, &data[at + header..(at + size).min(data.len())]));
        at += size;
    }
    out
}

/// The payload of the first box of `kind` in `data`.
fn child<'a>(data: &'a [u8], kind: &[u8; 4]) -> &'a [u8] {
    let found = boxes(data).into_iter().find(|(k, _)| k == kind).map(|(_, payload)| payload);
    found.unwrap_or_else(|| panic!("no {} box", String::from_utf8_lossy(kind)))
}

/// The chunk offsets of a sample table (`stco` or `co64`).
fn chunk_offsets(stbl: &[u8]) -> Vec<usize> {
    for (kind, body) in boxes(stbl) {
        if kind == *b"stco" {
            return (0..u32_at(body, 4) as usize).map(|i| u32_at(body, 8 + 4 * i) as usize).collect();
        }
        if kind == *b"co64" {
            return (0..u32_at(body, 4) as usize).map(|i| u64::from_be_bytes(body[8 + 8 * i..16 + 8 * i].try_into().unwrap()) as usize).collect();
        }
    }
    panic!("a sample table without chunk offsets")
}

/// The sample table of one `trak`.
fn track_table(trak: &[u8]) -> TrackTable {
    let mdia = child(trak, b"mdia");
    let handler: [u8; 4] = child(mdia, b"hdlr")[8..12].try_into().unwrap();
    let stbl = child(child(mdia, b"minf"), b"stbl");
    let stsz = child(stbl, b"stsz");
    let (fixed, count) = (u32_at(stsz, 4) as usize, u32_at(stsz, 8) as usize);
    let sizes: Vec<usize> = (0..count).map(|i| if fixed != 0 { fixed } else { u32_at(stsz, 12 + 4 * i) as usize }).collect();
    let stsc = child(stbl, b"stsc");
    let runs: Vec<(usize, usize)> = (0..u32_at(stsc, 4) as usize).map(|i| (u32_at(stsc, 8 + 12 * i) as usize, u32_at(stsc, 12 + 12 * i) as usize)).collect();
    let mut samples = Vec::with_capacity(count);
    for (chunk, &offset) in chunk_offsets(stbl).iter().enumerate() {
        let per_chunk = runs.iter().rev().find(|run| run.0 <= chunk + 1).map_or(0, |run| run.1);
        let mut at = offset;
        for _ in 0..per_chunk {
            let size = sizes[samples.len()];
            samples.push((at, size));
            at += size;
        }
    }
    TrackTable { handler, samples }
}

/// The track of `file` with the handler `handler`.
fn track(file: &[u8], handler: &[u8; 4]) -> TrackTable {
    for (kind, trak) in boxes(child(file, b"moov")) {
        if kind == *b"trak" {
            let table = track_table(trak);
            if table.handler == *handler {
                return table;
            }
        }
    }
    panic!("no {} track", String::from_utf8_lossy(handler))
}

// ------------------------------------------------------------------------------------ signals

/// The energy of `x` at `freq` Hz (Goertzel) at 48 kHz.
fn tone_power(x: &[f32], freq: f64) -> f64 {
    let coeff = 2.0 * (2.0 * std::f64::consts::PI * freq / 48_000.0).cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &v in x {
        let s0 = f64::from(v) + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    s1 * s1 + s2 * s2 - coeff * s1 * s2
}

/// CRC-32 of MPEG-2 sections (MSB first, polynomial 0x04C11DB7, initial value all ones).
fn crc32_mpeg(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ 0x04C1_1DB7 } else { crc << 1 };
        }
    }
    crc
}

/// The PID of the capture's programme map.
const PMT_PID: u16 = 0x0100;

/// The PID of the capture's sound.
const AUDIO_PID: u16 = 0x1100;

/// The programme association section (without its CRC): programme 1, its map at [`PMT_PID`].
const PAT: [u8; 12] = [0x00, 0xB0, 13, 0x00, 0x01, 0xC1, 0x00, 0x00, 0x00, 0x01, 0xE0 | (PMT_PID >> 8) as u8, PMT_PID as u8];

/// The programme map section (without its CRC): a Blu-ray registration (`HDMV`), so stream type
/// 0x80 is LPCM, and one LPCM stream at [`AUDIO_PID`].
const PMT: [u8; 23] = [
    0x02,
    0xB0,
    24,
    0x00,
    0x01,
    0xC1,
    0x00,
    0x00,
    0xE0 | (AUDIO_PID >> 8) as u8,
    AUDIO_PID as u8,
    0xF0,
    6,
    0x05,
    4,
    b'H',
    b'D',
    b'M',
    b'V',
    0x80,
    0xE0 | (AUDIO_PID >> 8) as u8,
    AUDIO_PID as u8,
    0xF0,
    0x00,
];

/// A PSI `section` with its CRC, in one transport packet of `pid` (pointer field 0, stuffed).
fn psi_packet(pid: u16, section: &[u8]) -> Vec<u8> {
    let mut packet = vec![0x47, 0x40 | (pid >> 8) as u8, pid as u8, 0x10, 0x00];
    packet.extend_from_slice(section);
    packet.extend_from_slice(&crc32_mpeg(section).to_be_bytes());
    packet.resize(188, 0xFF);
    packet
}

/// A PES packet of private stream 1 with `pts` (90 kHz) carrying `payload`.
fn pes(pts: u64, payload: &[u8]) -> Vec<u8> {
    let length = (3 + 5 + payload.len()) as u16;
    let mut packet = vec![0x00, 0x00, 0x01, 0xBD, (length >> 8) as u8, length as u8, 0x81, 0x80, 0x05];
    packet.push(0x21 | (((pts >> 30) & 0x07) << 1) as u8);
    packet.push((pts >> 22) as u8);
    packet.push(0x01 | (((pts >> 15) & 0x7F) << 1) as u8);
    packet.push((pts >> 7) as u8);
    packet.push(0x01 | ((pts & 0x7F) << 1) as u8);
    packet.extend_from_slice(payload);
    packet
}

/// The transport packets of `pid` carrying `pes` onto `out`, counting continuity in `cc`; the last
/// one is stuffed with an adaptation field.
fn packetize(pid: u16, pes: &[u8], cc: &mut u8, out: &mut Vec<u8>) {
    for (k, piece) in pes.chunks(184).enumerate() {
        let start = if k == 0 { 0x40 } else { 0x00 };
        out.extend_from_slice(&[0x47, start | (pid >> 8) as u8, pid as u8]);
        if piece.len() == 184 {
            out.push(0x10 | *cc);
        } else {
            // the adaptation field: its length byte, then flags and stuffing when there is room
            let stuffing = 184 - piece.len();
            out.push(0x30 | *cc);
            out.push((stuffing - 1) as u8);
            if stuffing > 1 {
                out.push(0x00);
                out.resize(out.len() + stuffing - 2, 0xFF);
            }
        }
        out.extend_from_slice(piece);
        *cc = (*cc + 1) & 0x0F;
    }
}

/// A Blu-ray style transport stream capture of `seconds` of 16-bit stereo LPCM at 48 kHz (a 440 Hz
/// tone left, 660 Hz right), one PES packet per 10 ms.
fn lpcm_capture(seconds: usize) -> Vec<u8> {
    let mut out = psi_packet(0x0000, &PAT);
    out.extend(psi_packet(PMT_PID, &PMT));
    let mut cc = 0u8;
    for unit in 0..seconds * 100 {
        // 1920 bytes of samples; stereo (3) at 48 kHz (1); 16 bits (1 << 6)
        let mut payload = vec![0x07, 0x80, 0x31, 0x40];
        for i in 0..480 {
            let t = (unit * 480 + i) as f64 / 48_000.0;
            for freq in [440.0, 660.0] {
                let v = (0.25 * (2.0 * std::f64::consts::PI * freq * t).sin() * 32_767.0) as i16;
                payload.extend_from_slice(&v.to_be_bytes());
            }
        }
        packetize(AUDIO_PID, &pes(90_000 + 900 * unit as u64, &payload), &mut cc, &mut out);
    }
    out
}

// ------------------------------------------------------------------------------------ tests

/// An intra-coded recording (ProRes, as the engine exports it) with one frame damaged in transfer:
/// that frame used to fail to decode and render as an empty frame, counted nowhere. It shows the
/// last good picture, every other frame is as in the undamaged file, and `metadata.get` counts the
/// concealed sample.
#[test]
fn a_damaged_intra_frame_shows_the_last_good_picture_and_is_counted() {
    let host = MemHost::new();
    let mut s = session(&host);
    let plasma = demo_footage(&mut s);
    place(&mut s, plasma, "V1", 25);
    let clean = export(&mut s, &host, "/out/plasma.mov", json!({"format": "prores", "audio": false}));
    let video = track(&clean, b"vide");
    assert_eq!(video.samples.len(), 25, "one ProRes frame per sequence frame");
    let mut damaged = clean.clone();
    let (at, _) = video.samples[10];
    // the frame identifier `icpf` after the frame size: the decoder refuses the frame
    damaged[at + 4..at + 8].copy_from_slice(b"XXXX");
    host.put("/media/clean.mov", clean);
    host.put("/media/damaged.mov", damaged);
    let (clean_frames, _, _) = imported(&host, "/media/clean.mov", 25);
    let (frames, mut d, item) = imported(&host, "/media/damaged.mov", 25);
    for k in (0..25).filter(|&k| k != 10) {
        assert!(frames[k] == clean_frames[k], "frame {k} is untouched by the damage");
    }
    assert!(frames[10] == frames[9], "the damaged frame shows the last good picture");
    let damage = damage(&mut d, item);
    assert_eq!(damage["videoSamplesConcealed"], json!(1), "the concealed sample is counted once: {damage}");
    assert_eq!(damage["audioPacketsConcealed"], json!(0), "{damage}");
}

/// An inter-coded recording (H.264 Baseline, a keyframe every 25 frames) with one predicted frame
/// damaged: the decoder used to stop at that sample on every request, so the rest of its GOP
/// rendered as empty frames. The damaged frame shows the last good picture, the frames after it
/// decode (from the references that are left) and move on, the next GOP is exact, and the
/// concealed sample is counted.
#[test]
fn a_damaged_inter_frame_does_not_freeze_the_rest_of_its_gop() {
    let host = MemHost::new();
    let mut s = session(&host);
    let plasma = demo_footage(&mut s);
    place(&mut s, plasma, "V1", 50);
    let params = json!({"format": "h264", "audio": false, "keyframeDistance": 25, "settings": {"h264Profile": "baseline"}});
    let clean = export(&mut s, &host, "/out/plasma.mp4", params);
    let video = track(&clean, b"vide");
    assert_eq!(video.samples.len(), 50, "one H.264 sample per sequence frame");
    let mut damaged = clean.clone();
    let (at, _) = video.samples[5];
    // a NAL length past the end of the sample: the decoder refuses the access unit
    damaged[at..at + 4].copy_from_slice(&[0xFF; 4]);
    host.put("/media/clean.mp4", clean);
    host.put("/media/damaged.mp4", damaged);
    let empty = empty_frame(&host);
    let (clean_frames, _, _) = imported(&host, "/media/clean.mp4", 50);
    let (frames, mut d, item) = imported(&host, "/media/damaged.mp4", 50);
    for k in 0..5 {
        assert!(frames[k] == clean_frames[k], "frame {k}, before the damage, is untouched");
    }
    assert!(frames[5] == frames[4], "the damaged frame shows the last good picture");
    let stuck: Vec<usize> = (6..25).filter(|&k| frames[k] == empty).collect();
    assert!(stuck.is_empty(), "frames {stuck:?} after the damage are empty: the GOP is stuck on the damage");
    assert!((7..25).any(|k| frames[k] != frames[k - 1]), "the frames after the damage move on: the GOP is not frozen");
    for k in 25..50 {
        assert!(frames[k] == clean_frames[k], "frame {k}, in the next GOP, is untouched");
    }
    let damage = damage(&mut d, item);
    assert_eq!(damage["videoSamplesConcealed"], json!(1), "the concealed sample is counted once: {damage}");
}

/// A recording whose AAC sound has one access unit damaged (it decodes to an error: a mono element
/// in a stereo stream). The mix stays finite, silence takes the packet's place as before, and
/// `metadata.get` now counts the concealed packet once, however often the packet is read.
#[test]
fn a_damaged_audio_packet_is_concealed_and_counted() {
    let host = MemHost::new();
    let mut s = session(&host);
    let plasma = demo_footage(&mut s);
    place(&mut s, plasma, "V1", 50);
    let params = json!({"format": "h264", "keyframeDistance": 25, "settings": {"h264Profile": "baseline"}});
    let clean = export(&mut s, &host, "/out/sound.mp4", params);
    let sound = track(&clean, b"soun");
    assert!(sound.samples.len() > 40, "two seconds of AAC: {} access units", sound.samples.len());
    let mut damaged = clean;
    let (at, size) = sound.samples[30];
    damaged[at..at + size].fill(0);
    host.put("/media/sound.mp4", damaged);
    let (_, mut d, item) = imported(&host, "/media/sound.mp4", 50);
    for _ in 0..2 {
        let mixed = mix(&d, 0, 96_000);
        assert!(mixed.iter().flatten().all(|v| v.is_finite()), "the sound stays finite");
    }
    let damage = damage(&mut d, item);
    assert_eq!(damage["audioPacketsConcealed"], json!(1), "the concealed packet is counted once: {damage}");
    assert_eq!(damage["videoSamplesConcealed"], json!(0), "{damage}");
}

/// A 5.1 source (a WAV in the engine's order, L, R, C, LFE, Ls, Rs; the order an AAC 5.1 source is
/// read in) on a stereo track used to play its first two channels only. It is downmixed with
/// ITU-R BS.775 (the default mixdown): each side has its front channel, the centre and the surround
/// of its side, and no LFE.
#[test]
fn a_51_source_on_a_stereo_track_is_downmixed() {
    let host = MemHost::new();
    let freqs = [300.0, 500.0, 1000.0, 80.0, 700.0, 900.0];
    let mut interleaved = Vec::with_capacity(48_000 * 6);
    for i in 0..48_000 {
        for &freq in &freqs {
            interleaved.push(0.2 * (2.0 * std::f64::consts::PI * freq * f64::from(i) / 48_000.0).sin() as f32);
        }
    }
    host.put("/media/six.wav", filmcraft_media::wav::write_wav16(&interleaved, 6, 48_000));
    let mut s = session(&host);
    let item = import(&mut s, "/media/six.wav");
    place(&mut s, item, "A1", 25);
    let stereo = mix(&s, 12_000, 4_800);
    let sides = [("left", &stereo[0], [300.0, 1000.0, 700.0], [500.0, 900.0, 80.0]), ("right", &stereo[1], [500.0, 1000.0, 900.0], [300.0, 700.0, 80.0])];
    for (side, x, present, absent) in sides {
        let top = freqs.iter().map(|&f| tone_power(x, f)).fold(0.0, f64::max);
        for freq in present {
            assert!(tone_power(x, freq) > top * 0.05, "the {side} side carries the {freq} Hz channel");
        }
        for freq in absent {
            assert!(tone_power(x, freq) < top * 0.001, "the {side} side carries no {freq} Hz");
        }
    }
}

/// An off-air transport stream capture (30 s of Blu-ray LPCM, 6 MB here, gigabytes on air) used to
/// be scanned whole by `file.import`, on the caller's thread. Its import now returns at once with
/// the item and the background job that indexes it, reading no more on the caller's thread than the
/// file's identity and its head; when the job is done, the item lasts the capture's length.
#[test]
fn a_transport_stream_capture_imports_in_the_background() {
    let host = MemHost::new();
    let capture = lpcm_capture(30);
    let size = capture.len();
    host.put("/media/capture.ts", capture);
    let mut s = session(&host);
    let before = host.read_on_caller();
    let imported = s.execute("file.import", json!({"paths": ["/media/capture.ts"]})).expect("the capture imports");
    let read = host.read_on_caller() - before;
    assert!(read < 3 << 20, "the import read {read} of {size} bytes on the caller's thread");
    let job = imported["jobs"][0].as_u64().unwrap_or_else(|| panic!("the import names its background job: {imported}"));
    let item = imported["items"][0].as_u64().unwrap_or_else(|| panic!("the item is there at once: {imported}"));
    let started = Instant::now();
    loop {
        s.poll_persistence();
        let jobs = s.execute("jobs.list", json!({})).expect("the jobs list");
        let entry = jobs.as_array().and_then(|all| all.iter().find(|j| j["id"] == json!(job)).cloned());
        let entry = entry.unwrap_or_else(|| panic!("the import job is listed: {jobs}"));
        if entry["finished"] == json!(true) {
            assert!(entry["result"].get("error").is_none(), "the import job succeeds: {entry}");
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(120), "the import job finishes");
        std::thread::sleep(Duration::from_millis(10));
    }
    s.poll_persistence();
    let duration = s.project.item(ItemId(item)).expect("the item").duration().seconds();
    assert!((duration - 30.0).abs() < 0.05, "the item lasts the capture's 30 s, not {duration} s");
}

/// A fast-start download cut at 60 % (as a browser leaves an interrupted download) used to import at
/// its full length, its last 40 % black and silent, with no warning. It imports as long as what is
/// there, and the Events log warns that the file is truncated.
#[test]
fn a_cut_faststart_download_imports_clamped_with_a_warning() {
    let host = MemHost::new();
    let mut s = session(&host);
    let plasma = demo_footage(&mut s);
    place(&mut s, plasma, "V1", 100);
    let params = json!({"format": "h264", "keyframeDistance": 25, "settings": {"h264Profile": "baseline"}});
    let whole = export(&mut s, &host, "/out/download.mp4", params);
    let order: Vec<[u8; 4]> = boxes(&whole).iter().map(|(kind, _)| *kind).collect();
    let (moov, mdat) = (order.iter().position(|k| k == b"moov"), order.iter().position(|k| k == b"mdat"));
    assert!(moov.is_some() && moov < mdat, "the export is fast start: {order:?}");
    host.put("/media/cut.mp4", whole[..whole.len() * 6 / 10].to_vec());
    let mut d = session(&host);
    let item = import(&mut d, "/media/cut.mp4");
    let duration = d.project.item(ItemId(item)).expect("the item").duration().seconds();
    assert!(duration > 0.0 && duration < 3.5, "the clip ends where the samples end, not at {duration} s of 4 s");
    let warned = d.log.entries.iter().any(|e| e.level == Level::Warning && e.message.contains("cut.mp4") && e.message.to_lowercase().contains("truncated"));
    assert!(warned, "the import warns that the file is truncated: {:?}", d.log.entries);
}
