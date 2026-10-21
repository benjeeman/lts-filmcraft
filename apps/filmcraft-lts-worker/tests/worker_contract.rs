//! T-NW: the contract of the LTS native worker of the Video editor engine (slice NW-1a, master plan
//! v3 W7; native-worker-architecture.md §5 and §8.4). The host drives the worker over EWP v1; these
//! tests drive it the same way, in this process through `Worker::handle` and, where a fresh process
//! or a kill matters, as the worker binary over its stdin and stdout.
//!
//! | Id | Test | What it pins |
//! |---|---|---|
//! | T-NW-SVC-01 | `contained_services_serve_only_staged_files` | staged files by their virtual name only; nothing listed |
//! | T-NW-SVC-02 | `staging_refuses_bytes_that_do_not_hash_to_their_name` | a staged name whose bytes hash otherwise is refused |
//! | T-NW-SVC-03 | `writes_land_only_in_the_project_and_outputs_folders` | exports land only under `outputs/` |
//! | T-NW-SVC-04 | `reparse_points_are_refused` | a symbolic link or a junction never stands in for a workspace file or folder |
//! | T-NW-ESC | `a_crafted_project_cannot_read_outside_the_workspace` | a project's outside media stay offline, unread |
//! | T-NW-POL-01 | `the_worker_table_is_the_hook_table` | the same policy file, the same sha, as the FF2 hook |
//! | T-NW-POL-02 | `raw_ids_tier3_and_the_browser_export_are_refused` | raw ids and tier 3 refused at the worker (T-FI-12) |
//! | T-NW-ENV-01 | `a_worker_never_scans_the_system_fonts` | OC-33 E1 at startup; software decoding; an unloaded family refused |
//! | T-NW-ENV-02 | `a_worker_process_never_scans_the_system_fonts` | the same in a fresh process with the ambient environment |
//! | T-NW-ENV-03 | `a_registered_hebrew_font_renders_hebrew` | `hello` registers a font by its bytes; Hebrew has its glyphs (ignored) |
//! | T-NW-ENV-04 | `fonts_are_registered_whole_or_not_at_all_and_substitution_is_labelled` | bad font files refused; `substituted` |
//! | T-NW-FR-01 | `worker_frames_equal_the_engine_frames_bit_for_bit` | frames are `try_render_program_at`'s; reads change nothing |
//! | T-NW-PRE | `stale_preconditions_are_refused_atomically` | rev, sequence, history head, linked selection: atomic refusals |
//! | T-NW-IMP-01 | `a_file_the_decoders_refuse_is_refused_before_the_import` | `format_unsupported` before dispatch; nothing fences |
//! | T-NW-IMP-02 | `warnings_and_damage_reach_the_caller` | the import's `media[]`; a frame's `concealed`; `partial` coverage |
//! | T-NW-IDX-01 | `a_background_index_is_a_job_of_the_worker` | `source_loading`; `revision`, checkpoint and journal; `jobResult` |
//! | T-NW-WAIT-01 | `a_scene_detection_runs_to_its_end_within_the_call` | `wait: false` refused; the cuts land in the call |
//! | T-NW-EWP-01 | `hello_limits_deadlines_and_line_sizes_hold` | limits, decode, deadlines, read kinds, line sizes, pong, build |
//! | T-NW-EXP-01 | `an_export_lands_under_outputs_and_repeats_byte_for_byte` | deterministic exports bound to the revision they render |
//! | T-NW-EXP-02 | `a_cancelled_export_leaves_no_partial_file` | cancel publishes nothing and leaves nothing |
//! | T-NW-EXP-03 | `a_stale_or_over_long_export_is_refused` | `revision_conflict`, `budget_exceeded`, `unsupported_operation` |
//! | T-NW-JRN-01 | `every_edit_is_journaled_before_and_after_it_runs` | `intent` before, `settled` after |
//! | T-NW-JRN-02 | `an_open_intent_settles_by_its_class` | a lost mutation `unknown` / `worker_lost`, a lost job `not_applied` |
//! | T-NW-JRN-03 | `a_worker_killed_mid_job_leaves_it_not_applied` | a real kill between a job's intent and its settlement |
//! | T-NW-JRN-04 | `a_worker_killed_mid_mutation_leaves_it_unknown` | a real kill while an edit runs |
//! | T-NW-CKP-01 | `checkpoints_of_two_workers_never_collide` | `rev-<n>-<sha12>.fcproj`, never replaced |
//! | T-NW-IMG-01 | `worker_images_carry_no_metadata` | fresh PNG and JPEG with allowed chunks and segments only |
//! | T-NW-IMG-02 | `planted_png_chunks_are_refused` | `tEXt`, `zTXt`, `iTXt`, `eXIf`, `iCCP` refused |
//! | T-NW-IMG-03 | `planted_jpeg_segments_are_refused` | APP1 (EXIF), APP2 (ICC) and comments refused |
//! | T-NW-IMG-04 | `mismatched_or_broken_images_are_refused` | dimensions other than expected, truncation, garbage |
//!
//! The journal and image checks are the worker's half: the host (NW-1b) settles a lost worker's journal with
//! `journal::recover` and checks every image with `imaging::check_png` / `check_jpeg` before a model sees it.
//!
//! EWP v1 as these tests send and expect it (host → worker, then worker → host):
//!
//! ```text
//! hello {protocol, mode, engine, sessionId, instanceId, limits?, fonts?, decode?}
//!     → ready {protocol, engine, build {policySha256, policyVersion, systemFontScan, hardwareDecoding, workerSha256,
//!       fontsetDigest, head, diffSha256}, operations[], readKinds[], jobKinds[], limits, fonts[], fontsetDigest}
//! stage {stageId, handle: "sha256:<hex>", path: "files/lts-<hex>.<ext>", bytes} → staged {stageId, ok, code?}
//! import {requestId, handle, expect, deadlineMs}  → result {requestId, envelope, guarantee: "atomic", media[], jobs[]?}
//! execute {requestId, operation, params, expect, deadlineMs}    → result {requestId, envelope, guarantee: "atomic"}
//! open {requestId, path: "project/<name>", expectSha256}  → result {envelope {result {unresolved[]}}, guarantee: "unavailable"}
//! read {readId, kind: "frames"|"fonts"|"query", params, deadlineMs} → readResult {readId, ok, value, files[]}
//! job {requestId, kind: "export_media", params, expect, deadlineMs} → jobAccepted {requestId, revision, sequence};
//!     later jobResult {requestId, kind, ok, code?, revision, outputs[]}
//! cancel {requestId}; ping {seq} → pong {seq, revision, queue, rssBytes}
//! ```
//!
//! `expect` is `{revision, activeSequence, historyHead?, linkedSelection?, targeting?}` (decimal strings, as the hook's
//! `executeTyped` takes them) and an envelope is the hook's own (`agent_view_policy::settle` and `refused`). Paths in
//! answers are relative to the session workspace. Media are made here (PNG stills, MP4, MOV and transport streams from
//! the workspace's own muxers and encoders); nothing is downloaded and no model is called. No test here registers a
//! font (T-NW-ENV-03, ignored, does): the font database is the process's, and T-NW-ENV-01 compares it.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use filmcraft_bitstream::BitWriter;
use filmcraft_engine::project::ItemId;
use filmcraft_engine::time::TICKS_PER_SECOND;
use filmcraft_engine::{FsServices, Services, Session};
use filmcraft_isobmff::{AvcConfig, Brand, FourCc, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};
use filmcraft_lts_worker::agent_view_policy as policy;
use filmcraft_lts_worker::{Worker, imaging, journal};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// The sha256 of the policy file that the FF2 hook build pinned (FF2-WASM-01, package
/// filmcraft-embed-20261014-agentview-v1, through the R1-F-GREEN-03 gate). The worker runs this very
/// table: a change to the table changes both builds, and this pin with them.
const HOOK_POLICY_SHA256: &str = "c05704e4875ca605694f0bb0113893890c205fa2c0404a8093bb7978fb395bf3";

/// A family no one registers.
const UNREGISTERED_FAMILY: &str = "No Such Family For The Worker";

/// The Hebrew face the harness ships for the LTS editors (Noto Sans Hebrew, OFL), by its sha256.
const HEBREW_FONT_SHA256: &str = "7ef36a2c3593758cdb622e1bdef4f84523e92fbc3ccc667438dd80ff54c2de88";

// ------------------------------------------------------------------------------------ workspaces

/// Workspaces made so far in this process.
static WORKSPACES: AtomicUsize = AtomicUsize::new(0);

/// A session workspace for one test, under Cargo's temporary folder of this target, removed when the
/// test ends (whatever its outcome).
struct Workspace(PathBuf);

impl Workspace {
    fn new(name: &str) -> Workspace {
        let n = WORKSPACES.fetch_add(1, Ordering::SeqCst);
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("nw1a-{}-{n}-{name}", std::process::id()));
        if dir.exists() {
            std::fs::remove_dir_all(&dir).expect("an old workspace is removed");
        }
        std::fs::create_dir_all(&dir).expect("the workspace is created");
        Workspace(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ------------------------------------------------------------------------------------ fixtures

/// The lowercase hex sha256 of `bytes`.
fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The hook's policy file, as the hook's build reads it.
fn hook_policy_bytes() -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../filmcraft-web/src/agent_view_policy.rs");
    std::fs::read(path).expect("the hook's policy file reads")
}

/// A `width` × `height` PNG whose every pixel `pixel` decides.
fn png(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let picture = image::RgbaImage::from_fn(width, height, |x, y| image::Rgba(pixel(x, y)));
    let mut bytes = Vec::new();
    picture.write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png).expect("the PNG encodes");
    bytes
}

/// A still with a gradient and a bright square, so that every frame showing it has content.
fn gradient(width: u32, height: u32) -> Vec<u8> {
    png(width, height, |x, y| {
        let square = (width / 4..width / 2).contains(&x) && (height / 4..height / 2).contains(&y);
        if square { [250, 240, 20, 255] } else { [(x * 255 / width) as u8, (y * 255 / height) as u8, 120, 255] }
    })
}

/// A `width` × `height` baseline JPEG of the gradient, made by the `image` crate (no worker code).
fn jpeg(width: u32, height: u32) -> Vec<u8> {
    let picture = image::RgbImage::from_fn(width, height, |x, y| image::Rgb([(x * 255 / width) as u8, (y * 255 / height) as u8, 90]));
    let mut bytes = Vec::new();
    picture.write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Jpeg).expect("the JPEG encodes");
    bytes
}

/// An SPS NAL unit (ITU-T H.264 §7.3.2.1.1) of a 16-pixel-wide High picture, 8-bit 4:2:0: progressive
/// frames, or field coded with MBAFF when `interlaced` (1080i broadcast, which the decoder refuses).
fn sps(interlaced: bool) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_bits(0x67, 8);
    w.write_bits(100, 8); // profile_idc: High
    w.write_bits(0, 8); // constraint flags
    w.write_bits(30, 8); // level 3.0
    w.write_ue(0); // seq_parameter_set_id
    w.write_ue(1); // chroma_format_idc: 4:2:0
    w.write_ue(0); // bit_depth_luma_minus8
    w.write_ue(0); // bit_depth_chroma_minus8
    w.write_bits(0, 2); // no transform bypass, no scaling matrices
    w.write_ue(0); // log2_max_frame_num_minus4
    w.write_ue(2); // pic_order_cnt_type
    w.write_ue(1); // max_num_ref_frames
    w.write_bit(false); // gaps_in_frame_num_value_allowed_flag
    w.write_ue(0); // pic_width_in_mbs_minus1
    w.write_ue(0); // pic_height_in_map_units_minus1
    w.write_bit(!interlaced); // frame_mbs_only_flag
    if interlaced {
        w.write_bit(true); // mb_adaptive_frame_field_flag
    }
    w.write_bit(true); // direct_8x8_inference_flag
    w.write_bit(false); // frame_cropping_flag
    w.write_bit(false); // vui_parameters_present_flag
    w.rbsp_trailing();
    w.finish()
}

/// A one-frame MP4 whose H.264 track carries `sps`; its sample is never decoded.
fn h264_mp4(sps: Vec<u8>) -> Vec<u8> {
    let pps = vec![0x68, 0xCE, 0x38, 0x80];
    let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).expect("a writer");
    let entry = SampleEntry::avc(AvcConfig::new(vec![sps], vec![pps], 4), 16, 32);
    let track = mux.add_track(TrackConfig::new(entry, 25)).expect("a track");
    mux.write_sample(track, WriteSample { data: &[0, 0, 0, 2, 0x65, 0x88], duration: 1, composition_offset: 0, is_sync: true }).expect("a sample");
    mux.finish().expect("the file").into_inner()
}

/// A 64×32 ProRes 422 HQ MOV at 25 fps, one sample per entry of `frames`: a picture whose luma (10
/// bits) is the entry, or, for `None`, bytes that are no ProRes frame at all (a sample damaged in
/// transfer); its index in front when `faststart`.
fn prores_mov(frames: &[Option<u16>], faststart: bool) -> Vec<u8> {
    let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mov)).expect("a writer");
    let video = mux.add_track(TrackConfig::new(SampleEntry::prores(FourCc(*b"apch"), 64, 32), 25)).expect("a video track");
    let mut encoder = filmcraft_prores::Encoder::new(filmcraft_prores::Profile::Hq, 64, 32);
    for &luma in frames {
        let data = match luma {
            Some(luma) => {
                let mut frame = filmcraft_prores::Frame::new(64, 32, filmcraft_prores::ChromaFormat::Yuv422, 10, false);
                frame.y.fill(luma);
                encoder.encode(&frame).expect("the frame encodes")
            }
            None => b"this is not a ProRes frame at all".to_vec(),
        };
        mux.write_sample(video, WriteSample { data: &data, duration: 1, composition_offset: 0, is_sync: true }).expect("a sample");
    }
    let file = if faststart { mux.finish_faststart() } else { mux.finish() };
    file.expect("the file").into_inner()
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

/// A PSI `section` with its CRC, in one transport packet of `pid` (pointer field 0, stuffed).
fn psi_packet(pid: u16, section: &[u8]) -> Vec<u8> {
    let mut packet = vec![0x47, 0x40 | (pid >> 8) as u8, pid as u8, 0x10, 0x00];
    packet.extend_from_slice(section);
    packet.extend_from_slice(&crc32_mpeg(section).to_be_bytes());
    packet.resize(188, 0xFF);
    packet
}

/// A PES packet of `stream_id` with `pts` (90 kHz) carrying `payload`.
fn pes(stream_id: u8, pts: u64, payload: &[u8]) -> Vec<u8> {
    let length = (3 + 5 + payload.len()) as u16;
    let mut packet = vec![0x00, 0x00, 0x01, stream_id, (length >> 8) as u8, length as u8, 0x81, 0x80, 0x05];
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

/// A Blu-ray style transport stream of `seconds` of a 440 Hz tone in 16-bit stereo LPCM at 48 kHz (its
/// programme map registered `HDMV`, so stream type 0x80 is LPCM), one PES packet per 10 ms from PTS 1 s:
/// about 207 kB a second, so three seconds and more are longer than the head the engine imports at
/// once (58 × 9024 bytes) and the engine indexes the rest in the background.
fn lpcm_stream(seconds: usize) -> Vec<u8> {
    const PMT_PID: u16 = 0x0100;
    const AUDIO_PID: u16 = 0x1100;
    let pat = [0x00, 0xB0, 13, 0x00, 0x01, 0xC1, 0x00, 0x00, 0x00, 0x01, 0xE0 | (PMT_PID >> 8) as u8, PMT_PID as u8];
    let mut body = vec![0x00, 0x01, 0xC1, 0x00, 0x00, 0xE0 | (AUDIO_PID >> 8) as u8, AUDIO_PID as u8, 0xF0, 6, 0x05, 4, b'H', b'D', b'M', b'V'];
    body.extend_from_slice(&[0x80, 0xE0 | (AUDIO_PID >> 8) as u8, AUDIO_PID as u8, 0xF0, 0x00]);
    let mut pmt = vec![0x02, 0xB0, (body.len() + 4) as u8];
    pmt.extend(body);
    let mut out = psi_packet(0x0000, &pat);
    out.extend(psi_packet(PMT_PID, &pmt));
    let mut cc = 0u8;
    for unit in 0..seconds * 100 {
        // 1920 bytes of samples; stereo (3) at 48 kHz (1); 16 bits (1 << 6)
        let mut payload = vec![0x07, 0x80, 0x31, 0x40];
        for i in 0..480 {
            let t = (unit * 480 + i) as f64 / 48_000.0;
            let [hi, lo] = ((0.25 * (2.0 * std::f64::consts::PI * 440.0 * t).sin() * 32_767.0) as i16).to_be_bytes();
            payload.extend_from_slice(&[hi, lo, hi, lo]);
        }
        packetize(AUDIO_PID, &pes(0xBD, 90_000 + 900 * unit as u64, &payload), &mut cc, &mut out);
    }
    out
}

/// A project made by an ordinary session: a 64×36 sequence at 25 fps with a title in `family` for a
/// second, encoded as a project file with its view. The session takes the family as it is named,
/// loaded or not; the process's guards go on first, so no lookup scans the system fonts.
fn titled_project(family: &str) -> Vec<u8> {
    filmcraft_lts_worker::startup();
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "Titled", "width": 64, "height": 36, "fps": 25})).expect("a sequence opens");
    s.execute("graphics.newText", json!({"text": "LTS", "font": family, "time": 0, "seconds": 1, "size": 16})).expect("the title is made");
    filmcraft_format::encode_with_view(&s.project, Some(&s.project_view()), false)
}

// ------------------------------------------------------------------------------------ EWP in this process

/// Milliseconds since the Unix epoch, the clock of an EWP deadline.
fn now_ms() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("the clock is past 1970").as_millis() as f64
}

/// A deadline a minute ahead.
fn deadline() -> f64 {
    now_ms() + 60_000.0
}

/// The one message `messages` holds.
fn one(messages: Vec<Value>) -> Value {
    assert_eq!(messages.len(), 1, "one answer expected: {messages:?}");
    messages.into_iter().next().expect("one answer")
}

/// The host's `hello`.
fn hello() -> Value {
    let protocol = filmcraft_lts_worker::PROTOCOL;
    json!({"type": "hello", "protocol": protocol, "mode": "session", "engine": "filmcraft", "sessionId": "s-nw1a", "instanceId": "i-nw1a"})
}

/// The host's `hello` with the keys of `extra` added or replaced (`limits`, `fonts`, `decode`).
fn hello_with(extra: Value) -> Value {
    let mut message = hello();
    if let (Some(keys), Value::Object(extra)) = (message.as_object_mut(), extra) {
        keys.extend(extra);
    }
    message
}

/// A worker over the workspace `ws`, after `hello`; answers it and its `ready`.
fn started(ws: &Workspace) -> (Worker, Value) {
    let (worker, ready) = started_with(ws, &hello());
    assert_eq!(ready["type"], json!("ready"), "hello is answered with ready: {ready}");
    (worker, ready)
}

/// A worker over the workspace `ws` that was sent `hello`; answers it and its answer to `hello`.
fn started_with(ws: &Workspace, hello: &Value) -> (Worker, Value) {
    let mut worker = Worker::new(ws.path()).expect("the worker starts");
    let answer = one(worker.handle(hello));
    (worker, answer)
}

/// What a caller binds an edit to right after reading the session: its revision and active sequence.
fn context(s: &Session) -> Value {
    json!({"revision": s.revision.to_string(), "activeSequence": s.state.active_sequence.map(|id| id.0.to_string())})
}

/// `execute` of a typed edit; answers the `result` message.
fn execute(worker: &mut Worker, request: &str, operation: &str, params: Value, expect: Value) -> Value {
    let message = json!({"type": "execute", "requestId": request, "operation": operation, "params": params, "expect": expect, "deadlineMs": deadline()});
    let answer = one(worker.handle(&message));
    assert_eq!(answer["type"], json!("result"), "`{operation}` is answered with its result: {answer}");
    assert_eq!(answer["requestId"], json!(request), "{answer}");
    answer
}

/// The engine's result of an applied edit, checked atomic.
fn applied(answer: &Value) -> Value {
    assert_eq!(answer["envelope"]["outcome"], json!("applied"), "{answer}");
    assert_eq!(answer["guarantee"], json!("atomic"), "{answer}");
    answer["envelope"]["result"].clone()
}

/// `execute` bound to the session as it is, applied; answers the engine's result.
fn edit(worker: &mut Worker, request: &str, operation: &str, params: Value) -> Value {
    let expect = context(worker.session());
    applied(&execute(worker, request, operation, params, expect))
}

/// Puts `bytes` in the workspace's `files/` under their content name, as the host does, and stages
/// them; answers their handle.
fn stage(worker: &mut Worker, bytes: &[u8], ext: &str) -> String {
    let sha = sha256_hex(bytes);
    let rel = format!("files/lts-{sha}.{ext}");
    std::fs::create_dir_all(worker.root().join("files")).expect("files/ exists");
    std::fs::write(worker.root().join(&rel), bytes).expect("the host places the file");
    let message = json!({"type": "stage", "stageId": format!("st-{}", &sha[..12]), "handle": format!("sha256:{sha}"), "path": rel, "bytes": bytes.len()});
    let answer = one(worker.handle(&message));
    assert_eq!(answer["type"], json!("staged"), "{answer}");
    assert_eq!(answer["ok"], json!(true), "the worker stages {rel}: {answer}");
    format!("sha256:{sha}")
}

/// The virtual path of a staged handle with extension `ext`.
fn staged_path(handle: &str, ext: &str) -> String {
    format!("/files/lts-{}.{ext}", handle.trim_start_matches("sha256:"))
}

/// Imports a staged file into the project (the worker's own typed operation); answers the item.
fn import(worker: &mut Worker, handle: &str) -> u64 {
    let expect = context(worker.session());
    let request = format!("imp-{}", &handle[7..19]);
    let message = json!({"type": "import", "requestId": request, "handle": handle, "expect": expect, "deadlineMs": deadline()});
    let answer = one(worker.handle(&message));
    assert_eq!(answer["type"], json!("result"), "{answer}");
    let result = applied(&answer);
    result["items"][0].as_u64().unwrap_or_else(|| panic!("one imported item: {result}"))
}

/// The cut the tests read: a 64×36 sequence at 25 fps, the gradient still on V1 for two seconds and
/// a title above it in the bundled face; answers the still's clip.
fn cut(worker: &mut Worker) -> u64 {
    edit(worker, "r-seq", "file.newSequence", json!({"name": "NW", "width": 64, "height": 36, "fps": 25}));
    let handle = stage(worker, &gradient(64, 36), "png");
    let item = import(worker, &handle);
    let placed = edit(worker, "r-place", "timeline.place", json!({"item": item, "track": "V1", "time": 0, "duration": 2 * TICKS_PER_SECOND}));
    edit(worker, "r-title", "graphics.newText", json!({"text": "LTS", "time": 0, "seconds": 2, "size": 16}));
    placed["clips"][0].as_u64().unwrap_or_else(|| panic!("the placed clip: {placed}"))
}

/// Frame `frame` of the active sequence, read through the worker at `max_side` as PNG; answers the
/// decoded pixels, the size and the `readResult`.
fn read_frame(worker: &mut Worker, frame: i64, max_side: u32) -> (Vec<u8>, u32, u32, Value) {
    let params = json!({"at": [{"frame": frame}], "maxSide": max_side, "format": "png"});
    let message = json!({"type": "read", "readId": format!("fr-{frame}-{max_side}"), "kind": "frames", "params": params, "deadlineMs": deadline()});
    let answer = one(worker.handle(&message));
    assert_eq!(answer["type"], json!("readResult"), "{answer}");
    assert_eq!(answer["ok"], json!(true), "frame {frame} reads: {answer}");
    let file = &answer["files"][0];
    let rel = file["path"].as_str().unwrap_or_else(|| panic!("a frame file: {answer}"));
    assert!(rel.starts_with("previews/"), "frames are written under previews/: {rel}");
    let bytes = std::fs::read(worker.root().join(rel)).expect("the frame file reads");
    assert_eq!(file["sha256"], json!(sha256_hex(&bytes)), "{answer}");
    assert_eq!(file["bytes"], json!(bytes.len()), "{answer}");
    let picture = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).expect("the frame is a PNG").to_rgba8();
    let (width, height) = picture.dimensions();
    (picture.into_raw(), width, height, answer)
}

/// A certified read through the worker (`read` kind `query`); answers the engine's result.
fn query(worker: &mut Worker, command: &str, params: Value) -> Value {
    let params = json!({"command": command, "params": params});
    let message = json!({"type": "read", "readId": format!("q-{command}"), "kind": "query", "params": params, "deadlineMs": deadline()});
    let answer = one(worker.handle(&message));
    assert_eq!(answer["type"], json!("readResult"), "{answer}");
    assert_eq!(answer["ok"], json!(true), "`{command}` reads: {answer}");
    answer["value"].clone()
}

/// Starts job `kind` under `request`, bound to the session as it is; the worker accepts it at once,
/// naming the revision it renders. Answers the `jobAccepted`.
fn start_job(worker: &mut Worker, request: &str, kind: &str, params: Value) -> Value {
    let expect = context(worker.session());
    let message = json!({"type": "job", "requestId": request, "kind": kind, "params": params, "expect": expect, "deadlineMs": deadline()});
    let answer = one(worker.handle(&message));
    assert_eq!(answer["type"], json!("jobAccepted"), "{answer}");
    assert_eq!(answer["requestId"], json!(request), "{answer}");
    assert_eq!(answer["revision"], json!(worker.session().revision.to_string()), "{answer}");
    answer
}

/// Sends job `kind` under `request`, bound to `expect`; answers the `jobResult` that refuses it at once.
fn job_refused(worker: &mut Worker, request: &str, kind: &str, params: Value, expect: Value) -> Value {
    let message = json!({"type": "job", "requestId": request, "kind": kind, "params": params, "expect": expect, "deadlineMs": deadline()});
    let answer = one(worker.handle(&message));
    assert_eq!(answer["type"], json!("jobResult"), "{answer}");
    assert_eq!(answer["ok"], json!(false), "{answer}");
    answer
}

/// The messages the worker's poll gives, polled as the host polls, up to the first that `last`
/// picks (`what`), that one included.
fn poll_until(worker: &mut Worker, what: &str, last: impl Fn(&Value) -> bool) -> Vec<Value> {
    let (started, mut seen) = (Instant::now(), Vec::new());
    while started.elapsed() < Duration::from_secs(600) {
        for message in worker.poll() {
            let done = last(&message);
            seen.push(message);
            if done {
                return seen;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("no {what} after 600 s: {seen:?}");
}

/// The result of job `request`, polled as the host polls.
fn job_result(worker: &mut Worker, request: &str) -> Value {
    let mut messages = poll_until(worker, request, |m| m["type"] == "jobResult" && m["requestId"] == request);
    messages.pop().expect("the job's result")
}

/// The worker's journal, one JSON object per line.
fn journal_lines(root: &Path) -> Vec<Value> {
    let text = std::fs::read_to_string(root.join("journal.jsonl")).expect("the worker keeps a journal");
    text.lines().map(|line| serde_json::from_str(line).expect("a journal line is JSON")).collect()
}

/// Whether the journal at `path` holds a line of type `t` for `request`.
fn journaled(path: &Path, t: &str, request: &str) -> bool {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines().any(|line| serde_json::from_str::<Value>(line).is_ok_and(|v| v["t"] == t && v["requestId"] == request))
}

// ------------------------------------------------------------------------------------ EWP over a process

/// A worker process driven over EWP as the host drives it: its stdin, its stdout read line by line
/// on a thread, and the revision and active sequence the last applied edit left.
struct Process {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    root: PathBuf,
    revision: String,
    sequence: Value,
}

impl Process {
    /// Starts the worker binary on the workspace `root`, with the ambient environment.
    fn spawn(root: &Path) -> Process {
        let mut command = Command::new(env!("CARGO_BIN_EXE_filmcraft-lts-worker"));
        command.arg("--workspace").arg(root).current_dir(root).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        let mut child = command.spawn().expect("the worker binary starts");
        let stdout = child.stdout.take().expect("the worker's stdout is piped");
        let stdin = child.stdin.take();
        let (sender, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Process { child, stdin, lines, root: root.to_path_buf(), revision: "1".to_string(), sequence: Value::Null }
    }

    fn send(&mut self, message: &Value) {
        let stdin = self.stdin.as_mut().expect("the worker's stdin is open");
        writeln!(stdin, "{message}").expect("the line reaches the worker");
        stdin.flush().expect("the line is flushed");
    }

    /// The next message that is not a job's progress.
    fn next(&self, what: &str) -> Value {
        loop {
            let line = self.lines.recv_timeout(Duration::from_secs(120)).unwrap_or_else(|e| panic!("no answer ({what}) from the worker in 120 s: {e}"));
            let message: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("the worker wrote a line that is not JSON ({e}): {line}"));
            if message["type"] != "progress" {
                return message;
            }
        }
    }

    /// `hello`, answered with `ready`.
    fn hello(&mut self) -> Value {
        self.send(&hello());
        let ready = self.next("ready");
        assert_eq!(ready["type"], json!("ready"), "hello is answered with ready: {ready}");
        ready
    }

    /// The result of an applied edit or import; the revision and sequence it leaves are kept.
    fn settle(&mut self, what: &str) -> Value {
        let answer = self.next(what);
        assert_eq!(answer["type"], json!("result"), "{answer}");
        let result = applied(&answer);
        self.revision = answer["envelope"]["revisionAfter"].as_str().unwrap_or_default().to_string();
        if let Some(sequence) = result.get("sequence").and_then(Value::as_u64) {
            self.sequence = json!(sequence.to_string());
        }
        result
    }

    /// Sends `execute` of `operation`, bound to the revision and sequence kept; its answer is read apart.
    fn send_edit(&mut self, request: &str, operation: &str, params: Value) {
        let expect = json!({"revision": self.revision, "activeSequence": self.sequence});
        self.send(&json!({"type": "execute", "requestId": request, "operation": operation, "params": params, "expect": expect, "deadlineMs": deadline()}));
    }

    /// Sends `job` of `kind`, bound to the revision and sequence kept; its answers are read apart.
    fn send_job(&mut self, request: &str, kind: &str, params: Value) {
        let expect = json!({"revision": self.revision, "activeSequence": self.sequence});
        self.send(&json!({"type": "job", "requestId": request, "kind": kind, "params": params, "expect": expect, "deadlineMs": deadline()}));
    }

    fn edit(&mut self, request: &str, operation: &str, params: Value) -> Value {
        self.send_edit(request, operation, params);
        self.settle(operation)
    }

    fn stage(&mut self, bytes: &[u8], ext: &str) -> String {
        let sha = sha256_hex(bytes);
        let rel = format!("files/lts-{sha}.{ext}");
        std::fs::create_dir_all(self.root.join("files")).expect("files/ exists");
        std::fs::write(self.root.join(&rel), bytes).expect("the host places the file");
        self.send(&json!({"type": "stage", "stageId": "st-process", "handle": format!("sha256:{sha}"), "path": rel, "bytes": bytes.len()}));
        let answer = self.next("staged");
        assert_eq!(answer["ok"], json!(true), "the worker stages {rel}: {answer}");
        format!("sha256:{sha}")
    }

    fn import(&mut self, handle: &str) -> u64 {
        let expect = json!({"revision": self.revision, "activeSequence": self.sequence});
        self.send(&json!({"type": "import", "requestId": "imp-process", "handle": handle, "expect": expect, "deadlineMs": deadline()}));
        let result = self.settle("import");
        result["items"][0].as_u64().unwrap_or_else(|| panic!("one imported item: {result}"))
    }

    fn read(&mut self, read_id: &str, kind: &str, params: Value) -> Value {
        self.send(&json!({"type": "read", "readId": read_id, "kind": kind, "params": params, "deadlineMs": deadline()}));
        let answer = self.next(read_id);
        assert_eq!(answer["type"], json!("readResult"), "{answer}");
        assert_eq!(answer["ok"], json!(true), "{read_id} reads: {answer}");
        answer["value"].clone()
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        self.kill();
    }
}

// ------------------------------------------------------------------------------------ T-NW-SVC

/// T-NW-SVC-01. The contained services serve a staged file by its virtual name, exactly (whole, by
/// range, through a reader, its size), and refuse every other path: absolute, drive, UNC and
/// verbatim paths (the staged file's own path on disk among them), `..` and `.` segments, names that
/// are not staged (a well-formed one included), other roots, a relative path and a staged name in
/// another case. Nothing is listed, and no volume or home folder is reported.
#[test]
fn contained_services_serve_only_staged_files() {
    let ws = Workspace::new("svc-01");
    let (mut worker, _) = started(&ws);
    let bytes = gradient(16, 8);
    let handle = stage(&mut worker, &bytes, "png");
    let staged = staged_path(&handle, "png");
    let sha = handle.trim_start_matches("sha256:").to_string();
    let services = worker.services();
    assert_eq!(services.read_file(&staged).expect("the staged file reads"), bytes);
    assert_eq!(services.file_size(&staged).expect("its size"), bytes.len() as u64);
    assert_eq!(services.read_range(&staged, 4, 8).expect("a range of it"), bytes[4..12].to_vec());
    let reader = services.reader(&staged).expect("a reader is offered").expect("the reader opens");
    assert_eq!(reader.len(), bytes.len() as u64);
    let mut head = [0u8; 8];
    reader.read_at(0, &mut head).expect("the reader reads");
    assert_eq!(head.as_slice(), &bytes[..8]);

    let other = sha256_hex(b"bytes that were never staged");
    let on_disk = worker.root().join(format!("files/lts-{sha}.png")).to_string_lossy().to_string();
    let refused = [
        "C:\\Windows\\win.ini".to_string(),
        "C:/Windows/win.ini".to_string(),
        "\\\\?\\C:\\Windows\\win.ini".to_string(),
        "\\\\localhost\\c$\\Windows\\win.ini".to_string(),
        "/etc/passwd".to_string(),
        on_disk,
        format!("/files/../files/lts-{sha}.png"),
        format!("/files/./lts-{sha}.png"),
        "/files/../journal.jsonl".to_string(),
        format!("/files/lts-{other}.png"),
        "/files/notes.txt".to_string(),
        format!("/projects/../files/lts-{sha}.png"),
        "/opfs/project.fcproj".to_string(),
        "/media/still.png".to_string(),
        format!("files/lts-{sha}.png"),
        format!("/FILES/lts-{sha}.png"),
        format!("/files/LTS-{}.PNG", sha.to_uppercase()),
    ];
    for path in &refused {
        assert!(services.read_file(path).is_err(), "{path} is refused");
        assert!(services.file_size(path).is_err(), "{path} has no size");
        assert!(services.read_range(path, 0, 4).is_err(), "{path} has no range");
        assert!(!matches!(services.reader(path), Some(Ok(_))), "{path} has no reader");
    }
    for dir in ["/", "/files", "/files/", "/projects", "/outputs", "C:\\", "C:\\Windows", ".", ""] {
        assert!(!matches!(services.list_dir(dir), Some(Ok(_))), "{dir:?} is not listed");
        assert!(!matches!(services.list_entries(dir), Some(Ok(_))), "{dir:?} has no entries");
    }
    assert!(services.volumes().is_empty(), "no volume is reported");
    assert!(services.home_dir().is_none(), "no home folder is reported");
    if let Some(loader) = services.file_loader() {
        assert!(loader("C:\\Windows\\win.ini").is_err(), "the frame loader is contained too");
    }
}

/// T-NW-SVC-02. Staging checks the bytes: a file whose bytes do not hash to its name is refused
/// `staged_file_corrupt`, so is a handle that names a hash other than the file's name and a name of
/// another form, and none of them can be read afterwards.
#[test]
fn staging_refuses_bytes_that_do_not_hash_to_their_name() {
    let ws = Workspace::new("svc-02");
    let (mut worker, _) = started(&ws);
    let good = gradient(16, 8);
    let handle = stage(&mut worker, &good, "png");
    let good_sha = handle.trim_start_matches("sha256:").to_string();

    let promised = sha256_hex(b"the bytes this name promises");
    let other = gradient(8, 8);
    std::fs::write(ws.path().join(format!("files/lts-{promised}.png")), &other).expect("other bytes under that name");
    let (claimed, at) = (format!("sha256:{promised}"), format!("files/lts-{promised}.png"));
    let message = json!({"type": "stage", "stageId": "st-corrupt", "handle": claimed, "path": at, "bytes": other.len()});
    let answer = one(worker.handle(&message));
    assert_eq!(answer["type"], json!("staged"), "{answer}");
    assert_eq!(answer["ok"], json!(false), "{answer}");
    assert_eq!(answer["code"], json!("staged_file_corrupt"), "{answer}");
    assert!(worker.services().read_file(&format!("/files/lts-{promised}.png")).is_err(), "a refused file is never read");

    let (claimed, at) = (format!("sha256:{promised}"), format!("files/lts-{good_sha}.png"));
    let message = json!({"type": "stage", "stageId": "st-other", "handle": claimed, "path": at, "bytes": good.len()});
    let answer = one(worker.handle(&message));
    assert_eq!(answer["ok"], json!(false), "a handle naming another hash than the file is refused: {answer}");

    for path in ["files/still.png", "files/lts-1234.png", "../outside.png", "files/sub/lts-x.png", "C:\\Windows\\win.ini", "project/rev-1.fcproj"] {
        let message = json!({"type": "stage", "stageId": "st-form", "handle": format!("sha256:{good_sha}"), "path": path, "bytes": good.len()});
        let answer = one(worker.handle(&message));
        assert_eq!(answer["ok"], json!(false), "{path} is not a staged name: {answer}");
        assert!(answer["code"].is_string(), "the refusal has a code: {answer}");
    }
}

/// T-NW-SVC-03. The engine's writes land only in the workspace's project folder, in an export's own
/// folder under `outputs/` and in the interchange folder mapped under `outputs/`; every other path
/// is refused and writes nothing: a staged file (read-only), a new name under `files/`, `..`
/// segments, the journal, an output without a request folder, an absolute path (one inside the
/// workspace included) and a relative one.
#[test]
fn writes_land_only_in_the_project_and_outputs_folders() {
    let ws = Workspace::new("svc-03");
    let (mut worker, _) = started(&ws);
    let bytes = gradient(8, 8);
    let handle = stage(&mut worker, &bytes, "png");
    let services = worker.services();
    services.write_file("/outputs/req-1/cut.mp4", b"export").expect("an export lands under outputs/");
    assert_eq!(std::fs::read(ws.path().join("outputs/req-1/cut.mp4")).expect("on disk"), b"export");
    services.write_file("/projects/rev-1.fcproj", b"{}").expect("a snapshot lands under project/");
    assert_eq!(std::fs::read(ws.path().join("project/rev-1.fcproj")).expect("on disk"), b"{}");
    assert_eq!(services.read_file("/projects/rev-1.fcproj").expect("the project folder reads"), b"{}");
    services.write_file("/lts-exports/cut.edl", b"TITLE: NW").expect("an interchange export lands under outputs/lts-exports/");
    assert_eq!(std::fs::read(ws.path().join("outputs/lts-exports/cut.edl")).expect("on disk"), b"TITLE: NW");

    let inside = ws.path().join("outputs").join("req-2").join("x.mp4").to_string_lossy().to_string();
    let refused = [
        staged_path(&handle, "png"),
        "/files/new.png".to_string(),
        "/outputs/../journal.jsonl".to_string(),
        "/outputs/req-1/../../journal.jsonl".to_string(),
        "/projects/../journal.jsonl".to_string(),
        "/journal.jsonl".to_string(),
        "/outputs/x.mp4".to_string(),
        "C:\\Windows\\Temp\\nw1a-x.mp4".to_string(),
        "/tmp/nw1a-x.mp4".to_string(),
        "outputs/req-1/x.mp4".to_string(),
        inside,
    ];
    for path in &refused {
        assert!(services.write_file(path, b"planted").is_err(), "{path} is not written");
    }
    assert_eq!(services.read_file(&staged_path(&handle, "png")).expect("the staged file still reads"), bytes);
    let journal_bytes = std::fs::read(ws.path().join("journal.jsonl")).unwrap_or_default();
    assert!(!journal_bytes.ends_with(b"planted"), "the journal was not written through the services");
    assert!(!ws.path().join("outputs").join("x.mp4").exists(), "no output outside a request folder");
    assert!(!ws.path().join("outputs").join("req-2").exists(), "an absolute path wrote nothing");
}

/// Makes `link` a junction to the folder `target` (no right to make symbolic links is needed for one).
#[cfg(windows)]
fn junction(link: &Path, target: &Path) {
    let made = Command::new("cmd").args(["/C", "mklink", "/J"]).arg(link).arg(target).stdout(Stdio::null()).status().expect("cmd runs");
    assert!(made.success(), "the junction {} is made", link.display());
}

/// T-NW-SVC-04. A reparse point never stands in for a workspace file or folder: a staged name that is
/// a symbolic link to a file outside the workspace is refused even though the bytes behind it hash to
/// its name; a `files/` folder that is a junction to a folder outside stages nothing, though the file
/// behind it hashes to its name too; and an outputs folder that is a junction to a folder outside
/// takes no export. Every case runs: a symbolic link that cannot be made (on Windows it needs
/// Developer Mode or elevation) fails the test instead of skipping its case.
#[cfg(windows)]
#[test]
fn reparse_points_are_refused() {
    let ws = Workspace::new("svc-04");
    let outside = Workspace::new("svc-04-outside");
    let (mut worker, _) = started(&ws);
    stage(&mut worker, &gradient(8, 8), "png");

    let secret = gradient(9, 9);
    let sha = sha256_hex(&secret);
    let target = outside.path().join("secret.png");
    std::fs::write(&target, &secret).expect("a file outside the workspace");
    let link = ws.path().join(format!("files/lts-{sha}.png"));
    std::os::windows::fs::symlink_file(&target, &link).expect("a symbolic link is made (Windows: Developer Mode or elevation)");
    let (claimed, at) = (format!("sha256:{sha}"), format!("files/lts-{sha}.png"));
    let message = json!({"type": "stage", "stageId": "st-link", "handle": claimed, "path": at, "bytes": secret.len()});
    let answer = one(worker.handle(&message));
    assert_eq!(answer["ok"], json!(false), "a symbolic link is not staged: {answer}");
    assert_eq!(answer["code"], json!("staged_file_corrupt"), "{answer}");
    assert!(worker.services().read_file(&format!("/files/lts-{sha}.png")).is_err(), "nothing behind the link is read");

    let hidden = gradient(10, 10);
    let hidden_sha = sha256_hex(&hidden);
    let elsewhere = outside.path().join("files");
    std::fs::create_dir_all(&elsewhere).expect("a folder outside the workspace");
    std::fs::write(elsewhere.join(format!("lts-{hidden_sha}.png")), &hidden).expect("a file in it, named by its hash");
    std::fs::remove_dir_all(ws.path().join("files")).expect("the files folder is removed");
    junction(&ws.path().join("files"), &elsewhere);
    let (claimed, at) = (format!("sha256:{hidden_sha}"), format!("files/lts-{hidden_sha}.png"));
    let message = json!({"type": "stage", "stageId": "st-junction", "handle": claimed, "path": at, "bytes": hidden.len()});
    let answer = one(worker.handle(&message));
    assert_eq!(answer["ok"], json!(false), "a file behind a junction is not staged: {answer}");
    assert_eq!(answer["code"], json!("staged_file_corrupt"), "{answer}");
    assert!(worker.services().read_file(&format!("/files/lts-{hidden_sha}.png")).is_err(), "nothing behind the junction is read");

    let outputs = ws.path().join("outputs");
    if outputs.exists() {
        std::fs::remove_dir_all(&outputs).expect("the outputs folder is removed");
    }
    junction(&outputs, outside.path());
    assert!(worker.services().write_file("/outputs/req-1/x.mp4", b"x").is_err(), "a junction takes no export");
    assert!(!outside.path().join("req-1").exists(), "nothing landed outside the workspace");
}

// ------------------------------------------------------------------------------------ T-NW-ESC

/// A project made by an ordinary session over the ambient file services: a 64×36 sequence at 25 fps
/// showing the picture at `path` for a second, encoded as a project file with its view.
fn crafted_project(path: &str) -> Vec<u8> {
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "Crafted", "width": 64, "height": 36, "fps": 25})).expect("a sequence opens");
    let imported = s.execute("file.import", json!({"paths": [path]})).expect("the ambient services import the outside picture");
    let item = imported["items"][0].as_u64().unwrap_or_else(|| panic!("one imported item: {imported}"));
    s.execute("timeline.place", json!({"item": item, "track": "V1", "time": 0, "duration": TICKS_PER_SECOND})).expect("the picture is placed");
    filmcraft_format::encode_with_view(&s.project, Some(&s.project_view()), false)
}

/// The project file `bytes` with the media path `from` replaced by `to`, as JSON strings.
fn repoint(bytes: &[u8], from: &str, to: &str) -> Vec<u8> {
    let text = String::from_utf8(bytes.to_vec()).expect("a project file is UTF-8 JSON");
    let (from, to) = (json!(from).to_string(), json!(to).to_string());
    assert!(text.contains(&from), "the project names {from}");
    text.replace(&from, &to).into_bytes()
}

/// T-NW-ESC. A project file that names media outside the workspace cannot make the worker read
/// them. Opened, the media stay offline: the frame shows no pixel of the outside picture (pure
/// green, which the offline slate never holds), and the open names each path it could not resolve,
/// `media_unresolved`. The outside picture is readable through the ambient file services, so the
/// trap is real; the second project names `C:\Windows\win.ini` itself.
#[test]
fn a_crafted_project_cannot_read_outside_the_workspace() {
    let ws = Workspace::new("esc");
    let outside = Workspace::new("esc-outside");
    let (mut worker, _) = started(&ws);
    let secret = png(32, 18, |_, _| [0, 255, 0, 255]);
    let trap = outside.path().join("Windows").join("win.ini");
    std::fs::create_dir_all(outside.path().join("Windows")).expect("an outside folder");
    std::fs::write(&trap, &secret).expect("an outside picture");
    let trap = trap.to_string_lossy().to_string();
    assert_eq!(FsServices.read_file(&trap).expect("the ambient services read it"), secret);
    let crafted = crafted_project(&trap);

    std::fs::create_dir_all(ws.path().join("project")).expect("project/ exists");
    for (name, path) in [("rev-1.fcproj", trap.clone()), ("rev-2.fcproj", "C:\\Windows\\win.ini".to_string())] {
        let bytes = repoint(&crafted, &trap, &path);
        std::fs::write(ws.path().join("project").join(name), &bytes).expect("the host places the project file");
        let message = json!({"type": "open", "requestId": format!("open-{name}"), "path": format!("project/{name}"), "expectSha256": sha256_hex(&bytes)});
        let answer = one(worker.handle(&message));
        assert_eq!(answer["type"], json!("result"), "{answer}");
        assert_eq!(answer["envelope"]["outcome"], json!("applied"), "{answer}");
        assert_eq!(answer["guarantee"], json!("unavailable"), "an open binds no precondition: {answer}");
        let result = &answer["envelope"]["result"];
        let named = result["unresolved"].as_array().is_some_and(|u| u.iter().any(|m| m["path"] == json!(path) && m["code"] == json!("media_unresolved")));
        assert!(named, "{name}: the open names {path} as unresolved: {result}");
        let (pixels, _, _, _) = read_frame(&mut worker, 0, 64);
        assert!(!pixels.chunks_exact(4).any(|p| p[0] < 40 && p[1] > 215 && p[2] < 40), "{name}: no pixel of the outside picture reached the frame");
    }
}

// ------------------------------------------------------------------------------------ T-NW-POL

/// T-NW-POL-01. The worker runs the hook's own table: `ready` reports the sha256 of the very policy
/// file the FF2 hook build pinned and its result version, and as its operations exactly the entries
/// of that table a native worker may run (the browser-only export left out, the native-only entries
/// in).
#[test]
fn the_worker_table_is_the_hook_table() {
    let ws = Workspace::new("pol-01");
    let (_worker, ready) = started(&ws);
    let file_sha = sha256_hex(&hook_policy_bytes());
    assert_eq!(file_sha, HOOK_POLICY_SHA256, "the hook's policy file is the one FF2 pinned");
    assert_eq!(ready["protocol"], json!(filmcraft_lts_worker::PROTOCOL), "{ready}");
    assert_eq!(ready["build"]["policySha256"], json!(file_sha), "the worker compiled the hook's file: {ready}");
    assert_eq!(ready["build"]["policyVersion"], json!(policy::RESULT_VERSION), "{ready}");
    let native: BTreeSet<&str> = policy::POLICY.iter().filter(|e| e.runtimes != policy::Runtimes::BrowserOnly).map(|e| e.command).collect();
    let advertised: BTreeSet<&str> = ready["operations"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    assert_eq!(advertised, native, "the worker offers exactly the table's native entries");
    assert!(native.contains("clip.sceneEditDetection") && !native.contains("file.exportMedia"));
    let kinds: BTreeSet<&str> = ready["readKinds"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    assert!(["frames", "fonts", "query"].iter().all(|k| kinds.contains(k)), "{ready}");
}

/// T-NW-POL-02 (reuses T-FI-12). Through the worker's `execute`, raw engine ids, tier-3 commands,
/// queries and keys outside an entry are `command_not_allowed`, the browser's media export is
/// `use_worker_export`, and nothing runs: the revision, the project and the undo stack stay as they
/// were. An edit of the table still applies afterwards.
#[test]
fn raw_ids_tier3_and_the_browser_export_are_refused() {
    let ws = Workspace::new("pol-02");
    let (mut worker, _) = started(&ws);
    let clip = cut(&mut worker);
    let refusals = [
        ("file.newProject", json!({}), "command_not_allowed"),
        ("file.open", json!({"path": "C:\\Windows\\win.ini"}), "command_not_allowed"),
        ("file.save", json!({}), "command_not_allowed"),
        ("file.saveAs", json!({"path": "/projects/x.fcproj"}), "command_not_allowed"),
        ("file.import", json!({"paths": ["C:\\Windows\\win.ini"]}), "command_not_allowed"),
        ("prefs.set", json!({"key": "autoSave", "value": false}), "command_not_allowed"),
        ("project.inspect", json!({}), "command_not_allowed"),
        ("file.exportFrame", json!({"path": "/outputs/x/frame.png"}), "command_not_allowed"),
        ("effects.toggleAnimation", json!({}), "command_not_allowed"),
        ("sequence.makeSubsequence", json!({}), "command_not_allowed"),
        ("transcript.downloadModel", json!({}), "command_not_allowed"),
        ("engine.execute", json!({"command": "file.save"}), "command_not_allowed"),
        ("timeline.trim", json!({"clip": clip, "delta": 0, "path": "C:\\x"}), "command_not_allowed"),
        ("file.exportMedia", json!({"path": "/lts-exports/cut.mp4", "format": "h264"}), "use_worker_export"),
    ];
    for (operation, params, code) in refusals {
        let s = worker.session();
        let (revision, project, undo) = (s.revision, Arc::clone(&s.project), s.history.undo.len());
        let expect = context(s);
        let answer = execute(&mut worker, &format!("raw-{operation}"), operation, params, expect);
        assert_eq!(answer["envelope"]["outcome"], json!("not_applied"), "{operation}: {answer}");
        assert_eq!(answer["envelope"]["code"], json!(code), "{operation}: {answer}");
        assert_eq!(answer["envelope"]["revisionAfter"], json!(revision.to_string()), "{operation}: {answer}");
        let s = worker.session();
        assert!(s.revision == revision && Arc::ptr_eq(&s.project, &project) && s.history.undo.len() == undo, "the refused `{operation}` ran nothing");
    }
    edit(&mut worker, "raw-after", "markers.add", json!({"time": 0}));
}

// ------------------------------------------------------------------------------------ T-NW-ENV

/// T-NW-ENV-01. A started worker has switched the system font scan off (OC-33 E1) and hardware
/// decoding off before anything runs. A title in a family no loaded face holds is refused
/// `font_not_loaded` before it runs (the render would substitute another face); resolving that
/// family for a coverage read, rendering a title in the bundled face and listing the fonts with
/// `system: true` leave the font families as they were, and no system scan happened.
#[test]
fn a_worker_never_scans_the_system_fonts() {
    let ws = Workspace::new("env-01");
    let (mut worker, ready) = started(&ws);
    assert!(!filmcraft_engine::graphics::system_scan_enabled(), "the worker switched the system font scan off");
    assert!(!filmcraft_codecs::hw::hardware_decoding(), "the worker decodes in software only");
    assert_eq!(ready["build"]["systemFontScan"], json!(false), "{ready}");
    assert_eq!(ready["build"]["hardwareDecoding"], json!(false), "{ready}");
    let families = filmcraft_text::fonts::families();
    edit(&mut worker, "env-seq", "file.newSequence", json!({"name": "ENV", "width": 64, "height": 36, "fps": 25}));
    let s = worker.session();
    let (revision, undo, expect) = (s.revision, s.history.undo.len(), context(s));
    let title = json!({"text": "LTS", "font": UNREGISTERED_FAMILY, "time": 0, "seconds": 1, "size": 16});
    let answer = execute(&mut worker, "env-title", "graphics.newText", title, expect);
    assert_eq!(answer["envelope"]["outcome"], json!("not_applied"), "{answer}");
    assert_eq!(answer["envelope"]["code"], json!("font_not_loaded"), "{answer}");
    let s = worker.session();
    assert!(s.revision == revision && s.history.undo.len() == undo, "the refused title ran nothing");
    let params = json!({"family": UNREGISTERED_FAMILY, "text": "LTS"});
    let covered = one(worker.handle(&json!({"type": "read", "readId": "env-fonts", "kind": "fonts", "params": params})));
    assert_eq!(covered["ok"], json!(true), "{covered}");
    assert_eq!(covered["value"]["coverage"]["loaded"], json!(false), "{covered}");
    edit(&mut worker, "env-title-inter", "graphics.newText", json!({"text": "LTS", "time": 0, "seconds": 1, "size": 16}));
    read_frame(&mut worker, 0, 64);
    let listed = query(&mut worker, "fonts.list", json!({"system": true}));
    assert!(listed.as_array().is_some_and(|f| f.iter().any(|f| f["family"] == json!("Inter"))), "the bundled fonts are listed: {listed}");
    assert_eq!(filmcraft_text::fonts::families(), families, "no family came from the system font folders");
    assert!(!filmcraft_text::fonts::system_scanned(), "the system font folders were never scanned");
}

/// T-NW-ENV-02. The same in a fresh worker process started with the ambient environment, where the
/// system font folders are there to be read: the font families the worker reports before and after a
/// title in an unregistered family is refused, that family is resolved for a coverage read, a title
/// is rendered and `fonts.list {system: true}` runs are the same, and no scan happened. `ready` names
/// the sha256 of the very binary that runs (`workerSha256`) and the font set's digest, which the
/// fonts read repeats.
#[test]
fn a_worker_process_never_scans_the_system_fonts() {
    let ws = Workspace::new("env-02");
    let mut worker = Process::spawn(ws.path());
    let ready = worker.hello();
    assert_eq!(ready["build"]["systemFontScan"], json!(false), "{ready}");
    let binary = std::fs::read(env!("CARGO_BIN_EXE_filmcraft-lts-worker")).expect("the worker binary reads");
    assert_eq!(ready["build"]["workerSha256"], json!(sha256_hex(&binary)), "ready names the binary's sha256: {ready}");
    let before = worker.read("fonts-0", "fonts", json!({}));
    assert!(before["families"].as_array().is_some_and(|f| !f.is_empty()), "the bundled families are there: {before}");
    assert_eq!(before["systemScanned"], json!(false), "{before}");
    assert_eq!(before["fontsetDigest"], ready["fontsetDigest"], "{before}");
    worker.edit("p-seq", "file.newSequence", json!({"name": "ENV", "width": 64, "height": 36, "fps": 25}));
    worker.send_edit("p-title", "graphics.newText", json!({"text": "LTS", "font": UNREGISTERED_FAMILY, "time": 0, "seconds": 1, "size": 16}));
    let refused = worker.next("p-title");
    assert_eq!(refused["envelope"]["code"], json!("font_not_loaded"), "{refused}");
    let covered = worker.read("fonts-cover", "fonts", json!({"family": UNREGISTERED_FAMILY, "text": "LTS"}));
    assert_eq!(covered["coverage"]["loaded"], json!(false), "{covered}");
    worker.edit("p-title-inter", "graphics.newText", json!({"text": "LTS", "time": 0, "seconds": 1, "size": 16}));
    worker.read("frame-0", "frames", json!({"at": [{"frame": 0}], "maxSide": 64, "format": "png"}));
    worker.read("fonts-list", "query", json!({"command": "fonts.list", "params": {"system": true}}));
    let after = worker.read("fonts-1", "fonts", json!({}));
    assert_eq!(after["families"], before["families"], "no family came from the system font folders");
    assert_eq!(after["systemScanned"], json!(false), "the system font folders were never scanned: {after}");
}

/// T-NW-ENV-03 (RD-14, Hebrew). `hello` registers a font file by its bytes under a name a call gives
/// for its family (`noto-sans-hebrew`), and `ready` lists it with the font set's digest, which a frames
/// read repeats. A Hebrew title in that name applies, in the family; every character of it has a
/// glyph in the face; nothing is substituted and no system font folder is scanned. Ignored in the
/// default run: it reads the font the LTS harness ships (`lts-dev/web/creative-fonts/NotoSansHebrew.ttf`
/// beside this checkout), which the fork does not carry, and it registers a face in the process's font
/// database, which T-NW-ENV-01 compares; run it on its own (`--ignored --exact`).
#[test]
#[ignore = "reads the LTS harness's Hebrew font and registers it process-wide: run it alone with --ignored --exact"]
fn a_registered_hebrew_font_renders_hebrew() {
    let font = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../../editor-harness-20261014/lts-dev/web/creative-fonts/NotoSansHebrew.ttf");
    let bytes = std::fs::read(&font).unwrap_or_else(|e| panic!("the harness font {} reads: {e}", font.display()));
    assert_eq!(sha256_hex(&bytes), HEBREW_FONT_SHA256, "the harness font is the one pinned");
    let ws = Workspace::new("env-03");
    std::fs::create_dir_all(ws.path().join("files")).expect("files/ exists");
    let rel = format!("files/lts-{HEBREW_FONT_SHA256}.ttf");
    std::fs::write(ws.path().join(&rel), bytes).expect("the host places the font");
    let fonts = json!([{"name": "noto-sans-hebrew", "path": rel, "sha256": HEBREW_FONT_SHA256}]);
    let (mut worker, ready) = started_with(&ws, &hello_with(json!({"fonts": fonts})));
    assert_eq!(ready["type"], json!("ready"), "{ready}");
    assert_eq!(ready["fonts"][0]["name"], json!("noto-sans-hebrew"), "{ready}");
    assert_eq!(ready["fonts"][0]["family"], json!("Noto Sans Hebrew"), "{ready}");
    assert_eq!(ready["fonts"][0]["sha256"], json!(HEBREW_FONT_SHA256), "{ready}");
    let digest = ready["fontsetDigest"].clone();
    assert!(digest.as_str().is_some_and(|d| d.starts_with("sha256:")), "{ready}");
    assert_eq!(ready["build"]["fontsetDigest"], digest, "{ready}");

    let text = "שלום עולם";
    edit(&mut worker, "he-seq", "file.newSequence", json!({"name": "HE", "width": 320, "height": 180, "fps": 25}));
    edit(&mut worker, "he-title", "graphics.newText", json!({"text": text, "font": "noto-sans-hebrew", "time": 0, "seconds": 1, "size": 48}));
    let project = serde_json::to_string(&*worker.session().project).expect("the project serializes");
    assert!(project.contains("Noto Sans Hebrew") && !project.contains("noto-sans-hebrew"), "the title names the family, not the file's name");
    let params = json!({"family": "Noto Sans Hebrew", "text": text});
    let covered = one(worker.handle(&json!({"type": "read", "readId": "he-fonts", "kind": "fonts", "params": params})));
    assert_eq!(covered["value"]["coverage"]["loaded"], json!(true), "{covered}");
    assert_eq!(covered["value"]["coverage"]["missing"], json!([]), "every Hebrew character has a glyph: {covered}");
    let (pixels, _, _, frame) = read_frame(&mut worker, 0, 320);
    assert_eq!(frame["value"]["fontsetDigest"], digest, "{frame}");
    assert_eq!(frame["value"]["substituted"], json!([]), "{frame}");
    assert!(pixels.chunks_exact(4).any(|p| p[0] > 200 && p[1] > 200 && p[2] > 200), "the title is drawn");
    assert!(!filmcraft_text::fonts::system_scanned(), "no system font folder was scanned");
}

/// T-NW-ENV-04 (RD-14). Fonts are registered whole or not at all: a `hello` naming a file that holds no
/// font face, a file whose bytes are not there under their hash, a name that is empty, a path that is
/// not a font file's or a list that is not one is refused (`ready` is not sent, nothing is
/// registered), and a `hello` without them then starts the session. A project made elsewhere whose
/// title names a family no loaded face holds opens (`guarantee: "unavailable"`) and renders in the
/// default face, and the open and every frames read name the family as `substituted`.
#[test]
fn fonts_are_registered_whole_or_not_at_all_and_substitution_is_labelled() {
    let ws = Workspace::new("env-04");
    std::fs::create_dir_all(ws.path().join("files")).expect("files/ exists");
    let not_a_font = gradient(8, 8);
    let sha = sha256_hex(&not_a_font);
    std::fs::write(ws.path().join(format!("files/lts-{sha}.ttf")), not_a_font).expect("the host places a file");
    let missing = sha256_hex(b"a font file the host never placed");
    let entry = |name: &str, path: String, sha256: &str| json!({"name": name, "path": path, "sha256": sha256});
    let refused = [
        ("no face", json!([entry("not-a-font", format!("files/lts-{sha}.ttf"), &sha)]), "invalid_params"),
        ("not there", json!([entry("missing", format!("files/lts-{missing}.ttf"), &missing)]), "staged_file_corrupt"),
        ("no name", json!([entry("", format!("files/lts-{sha}.ttf"), &sha)]), "invalid_params"),
        ("not a font file", json!([entry("png", format!("files/lts-{sha}.png"), &sha)]), "invalid_params"),
        ("not a list", json!({"name": "x"}), "invalid_params"),
    ];
    let families = filmcraft_text::fonts::families();
    let mut worker = Worker::new(ws.path()).expect("the worker starts");
    for (case, fonts, code) in refused {
        let answer = one(worker.handle(&hello_with(json!({"fonts": fonts}))));
        assert_eq!(answer["type"], json!("error"), "{case}: {answer}");
        assert_eq!(answer["code"], json!(code), "{case}: {answer}");
    }
    assert_eq!(filmcraft_text::fonts::families(), families, "no face was registered");
    let ready = one(worker.handle(&hello()));
    assert_eq!(ready["type"], json!("ready"), "a hello without fonts starts the session: {ready}");
    assert_eq!(ready["fonts"], json!([]), "{ready}");

    let titled = titled_project(UNREGISTERED_FAMILY);
    std::fs::create_dir_all(ws.path().join("project")).expect("project/ exists");
    std::fs::write(ws.path().join("project/titled.fcproj"), &titled).expect("the host places the project file");
    let message = json!({"type": "open", "requestId": "open-titled", "path": "project/titled.fcproj", "expectSha256": sha256_hex(&titled)});
    let answer = one(worker.handle(&message));
    assert_eq!(answer["envelope"]["outcome"], json!("applied"), "{answer}");
    assert_eq!(answer["guarantee"], json!("unavailable"), "{answer}");
    assert_eq!(answer["substituted"], json!([UNREGISTERED_FAMILY]), "{answer}");
    let (pixels, _, _, frame) = read_frame(&mut worker, 0, 64);
    assert_eq!(frame["value"]["substituted"], json!([UNREGISTERED_FAMILY]), "{frame}");
    assert_eq!(frame["value"]["fontsetDigest"], ready["fontsetDigest"], "{frame}");
    assert!(pixels.chunks_exact(4).any(|p| p[0] > 200 && p[1] > 200 && p[2] > 200), "the title renders in the default face");
}

// ------------------------------------------------------------------------------------ T-NW-FR-01

/// T-NW-FR-01. Worker frames are the engine's own: each frame the worker writes decodes to exactly
/// the pixels `Session::try_render_program_at` renders for that frame at that scale (over black, 8
/// bits), at full and at half size; the frame entry names the frame and its tick; and reading moves
/// neither the playhead nor the selection, nor anything the revision or the undo stack counts.
#[test]
fn worker_frames_equal_the_engine_frames_bit_for_bit() {
    let ws = Workspace::new("fr-01");
    let (mut worker, _) = started(&ws);
    cut(&mut worker);
    let s = worker.session();
    let before = (s.revision, s.playhead(), s.state.selection.clone(), s.history.undo.len());
    let rate = s.sequence_rate();
    for (frame, max_side) in [(0i64, 64u32), (7, 64), (30, 32), (49, 64)] {
        let scale = (max_side as f32 / 64.0).min(1.0);
        let engine = worker.session().try_render_program_at(scale, rate.tick_of(frame)).expect("the engine renders the frame");
        let (pixels, width, height, answer) = read_frame(&mut worker, frame, max_side);
        assert_eq!((width as usize, height as usize), (engine.w, engine.h), "frame {frame} at {max_side}");
        assert!(pixels == engine.over_black_rgba8(), "frame {frame} at {max_side} is the engine's frame, bit for bit");
        let entry = &answer["value"]["frames"][0];
        assert_eq!(entry["actual"]["frame"], json!(frame), "{answer}");
        assert_eq!(entry["actual"]["tick"], json!(rate.tick_of(frame).0.to_string()), "{answer}");
    }
    let s = worker.session();
    assert_eq!((s.revision, s.playhead(), s.state.selection.clone(), s.history.undo.len()), before, "reading frames changed nothing");
}

// ------------------------------------------------------------------------------------ T-NW-PRE

/// T-NW-PRE. Natively every precondition is checked in the same call as the edit (MC §9.2): a stale
/// revision, another active sequence, a stale history head and a stale linked-selection setting are
/// each refused `not_applied` with their code and `guarantee: "atomic"` before anything runs; the
/// same edit bound to the current context then applies, atomically too.
#[test]
fn stale_preconditions_are_refused_atomically() {
    let ws = Workspace::new("pre");
    let (mut worker, _) = started(&ws);
    cut(&mut worker);
    let s = worker.session();
    let (revision, sequence, linked) = (s.revision, s.state.active_sequence.map(|id| id.0.to_string()), s.state.linked_selection);
    let label = s.history.undo.last().map(|(label, _)| label.clone()).unwrap_or_default();
    let head = json!({"length": s.history.undo.len(), "label": label});
    let other_head = json!({"length": s.history.undo.len() + 1, "label": "Not the head"});
    let stale = [
        ("revision_conflict", json!({"revision": (revision - 1).to_string(), "activeSequence": sequence})),
        ("sequence_changed", json!({"revision": revision.to_string(), "activeSequence": "999999999"})),
        ("history_head_conflict", json!({"revision": revision.to_string(), "activeSequence": sequence, "historyHead": other_head})),
        ("linked_selection_changed", json!({"revision": revision.to_string(), "activeSequence": sequence, "linkedSelection": !linked})),
    ];
    for (code, expect) in stale {
        let s = worker.session();
        let (project, undo, journal_len) = (Arc::clone(&s.project), s.history.undo.len(), s.journal.len());
        let answer = execute(&mut worker, &format!("pre-{code}"), "markers.add", json!({"time": 0}), expect);
        let envelope = &answer["envelope"];
        assert_eq!(envelope["outcome"], json!("not_applied"), "{answer}");
        assert_eq!(envelope["code"], json!(code), "{answer}");
        assert_eq!(envelope["revisionBefore"], json!(revision.to_string()), "{answer}");
        assert_eq!(envelope["revisionAfter"], json!(revision.to_string()), "{answer}");
        assert_eq!(answer["guarantee"], json!("atomic"), "{answer}");
        let s = worker.session();
        assert!(s.revision == revision && Arc::ptr_eq(&s.project, &project), "the edit refused {code} ran nothing");
        assert!(s.history.undo.len() == undo && s.journal.len() == journal_len, "the edit refused {code} took no step");
    }
    let current = json!({"revision": revision.to_string(), "activeSequence": sequence, "historyHead": head, "linkedSelection": linked});
    let answer = execute(&mut worker, "pre-current", "markers.add", json!({"time": 0}), current);
    applied(&answer);
    assert_eq!(answer["envelope"]["revisionAfter"], json!((revision + 1).to_string()), "{answer}");
}

// ------------------------------------------------------------------------------------ T-NW-IMP, IDX, WAIT

/// T-NW-IMP-01. A file the decoders refuse is refused before the import runs: an interlaced H.264 MP4
/// is `not_applied`, `format_unsupported`, saying why (`detail`), with nothing changed and its
/// journal line `not_applied`; it never settles `unknown`, so nothing fences and the session goes on.
/// A progressive one imports, and its result names for its item the warnings opening its file gave
/// and the damage its source counted so far (`media`).
#[test]
fn a_file_the_decoders_refuse_is_refused_before_the_import() {
    let ws = Workspace::new("imp-01");
    let (mut worker, _) = started(&ws);
    edit(&mut worker, "i-seq", "file.newSequence", json!({"name": "IMP", "width": 64, "height": 36, "fps": 25}));
    let handle = stage(&mut worker, &h264_mp4(sps(true)), "mp4");
    let s = worker.session();
    let (revision, project, undo, expect) = (s.revision, Arc::clone(&s.project), s.history.undo.len(), context(s));
    let message = json!({"type": "import", "requestId": "imp-interlaced", "handle": handle, "expect": expect, "deadlineMs": deadline()});
    let answer = one(worker.handle(&message));
    let envelope = &answer["envelope"];
    assert_eq!(envelope["outcome"], json!("not_applied"), "{answer}");
    assert_eq!(envelope["code"], json!("format_unsupported"), "{answer}");
    assert!(envelope["detail"].as_str().is_some_and(|why| why.contains("interlaced")), "the refusal says why: {answer}");
    assert_eq!(envelope["revisionAfter"], json!(revision.to_string()), "{answer}");
    assert_eq!(answer["guarantee"], json!("atomic"), "{answer}");
    let s = worker.session();
    assert!(s.revision == revision && Arc::ptr_eq(&s.project, &project) && s.history.undo.len() == undo, "the refused import ran nothing");
    let lines = journal_lines(ws.path());
    let settled = lines.iter().find(|l| l["t"] == "settled" && l["requestId"] == "imp-interlaced").expect("the import is settled");
    assert_eq!(settled["outcome"], json!("not_applied"), "{settled}");
    assert_eq!(settled["code"], json!("format_unsupported"), "{settled}");

    let handle = stage(&mut worker, &h264_mp4(sps(false)), "mp4");
    let expect = context(worker.session());
    let message = json!({"type": "import", "requestId": "imp-progressive", "handle": handle, "expect": expect, "deadlineMs": deadline()});
    let answer = one(worker.handle(&message));
    let item = applied(&answer)["items"][0].as_u64().unwrap_or_else(|| panic!("one imported item: {answer}"));
    assert_eq!(answer["media"][0]["item"], json!(item), "{answer}");
    assert!(answer["media"][0]["warnings"].is_array(), "{answer}");
    assert_eq!(answer["media"][0]["damage"]["videoSamplesConcealed"], json!(0), "{answer}");
    edit(&mut worker, "i-after", "markers.add", json!({"time": 0}));
}

/// T-NW-IMP-02 (media-agent-contracts.md §8.4). What a source reports reaches the caller. A fast-start
/// MOV cut short imports with the warning its opening gave in its `media` entry. A MOV whose fifth
/// sample is damaged imports with no damage counted yet (nothing is decoded at import); a frame before
/// the damage is read whole; the damaged frame shows the last good picture, its entry names the item
/// as `concealed`, and the read's coverage is `partial` (`source_damaged`) with what the source
/// concealed.
#[test]
fn warnings_and_damage_reach_the_caller() {
    let ws = Workspace::new("imp-02");
    let (mut worker, _) = started(&ws);
    edit(&mut worker, "d-seq", "file.newSequence", json!({"name": "Damage", "width": 64, "height": 32, "fps": 25}));
    let ramp: Vec<Option<u16>> = (0..50u16).map(|k| Some(100 + 15 * k)).collect();
    let whole = prores_mov(&ramp, true);
    let handle = stage(&mut worker, &whole[..whole.len() * 6 / 10], "mov");
    let expect = context(worker.session());
    let message = json!({"type": "import", "requestId": "imp-cut", "handle": handle, "expect": expect, "deadlineMs": deadline()});
    let answer = one(worker.handle(&message));
    applied(&answer);
    let warnings = &answer["media"][0]["warnings"];
    assert!(warnings.as_array().is_some_and(|w| w.iter().any(|w| w.as_str().is_some_and(|w| w.contains("truncated")))), "{answer}");

    let frames: Vec<Option<u16>> = (0..10u16).map(|k| if k == 4 { None } else { Some(100 + 70 * k) }).collect();
    let handle = stage(&mut worker, &prores_mov(&frames, false), "mov");
    let expect = context(worker.session());
    let message = json!({"type": "import", "requestId": "imp-damaged", "handle": handle, "expect": expect, "deadlineMs": deadline()});
    let answer = one(worker.handle(&message));
    let item = applied(&answer)["items"][0].as_u64().unwrap_or_else(|| panic!("one imported item: {answer}"));
    assert_eq!(answer["media"][0]["item"], json!(item), "{answer}");
    assert_eq!(answer["media"][0]["damage"]["videoSamplesConcealed"], json!(0), "nothing is decoded at import: {answer}");
    edit(&mut worker, "d-place", "timeline.place", json!({"item": item, "track": "V1", "time": 0, "duration": 10 * TICKS_PER_SECOND / 25}));
    let (_, _, _, clean) = read_frame(&mut worker, 2, 64);
    assert!(clean["value"]["frames"][0].get("concealed").is_none(), "frame 2 shows no concealed sample: {clean}");
    assert!(clean["value"].get("coverage").is_none(), "{clean}");
    let (_, _, _, damaged) = read_frame(&mut worker, 4, 64);
    assert_eq!(damaged["value"]["frames"][0]["concealed"], json!([item]), "frame 4 shows the damaged sample concealed: {damaged}");
    let coverage = &damaged["value"]["coverage"];
    assert_eq!(coverage["status"], json!("partial"), "{damaged}");
    assert_eq!(coverage["why"], json!("source_damaged"), "{damaged}");
    assert_eq!(coverage["media"][0]["item"], json!(item), "{damaged}");
    assert!(coverage["media"][0]["damage"]["videoSamplesConcealed"].as_u64().is_some_and(|n| n >= 1), "{damaged}");
}

/// T-NW-IDX-01. An MPEG stream longer than the head the engine reads at once imports at once with what
/// its head holds, and the engine indexes the whole stream in the background: a job of the worker
/// (`index`, named in the import's `jobs`, journaled as a job). While it runs, an export is refused
/// `source_loading` (it would render the head only, and the engine would wait for the index on the
/// worker's thread). When the index is in, the project's revision moves between two requests: the
/// worker writes the new revision as a checkpoint, journals it and says so (`revision`, its cause the
/// index) before the index's `jobResult`; the item then lasts as long as the whole stream, and nothing
/// is left for the host to settle.
#[test]
fn a_background_index_is_a_job_of_the_worker() {
    let ws = Workspace::new("idx-01");
    let (mut worker, _) = started(&ws);
    edit(&mut worker, "x-seq", "file.newSequence", json!({"name": "IDX", "width": 64, "height": 36, "fps": 25}));
    let stream = lpcm_stream(4);
    assert!(stream.len() > 58 * 9024, "the stream is longer than the head the engine reads at once");
    let handle = stage(&mut worker, &stream, "ts");
    let expect = context(worker.session());
    let message = json!({"type": "import", "requestId": "imp-ts", "handle": handle, "expect": expect, "deadlineMs": deadline()});
    let answer = one(worker.handle(&message));
    let item = applied(&answer)["items"][0].as_u64().unwrap_or_else(|| panic!("one imported item: {answer}"));
    let job = &answer["jobs"][0];
    assert_eq!(job["kind"], json!("index"), "{answer}");
    assert_eq!(job["item"], json!(item), "{answer}");
    let index = job["requestId"].as_str().unwrap_or_else(|| panic!("the index job's id: {answer}")).to_string();
    assert!(index.starts_with("imp-ts/index-"), "{index}");
    let seconds = |worker: &Worker| {
        let media = worker.session().project.item(ItemId(item)).and_then(|it| it.as_media()).map(|m| m.info.duration.seconds());
        media.unwrap_or_default()
    };
    assert!(seconds(&worker) < 3.0, "the item holds the stream's head at first: {} s", seconds(&worker));

    let expect = context(worker.session());
    let refused = job_refused(&mut worker, "exp-early", "export_media", json!({"format": "h264"}), expect);
    assert_eq!(refused["code"], json!("source_loading"), "{refused}");
    assert_eq!(refused["revision"], json!(worker.session().revision.to_string()), "{refused}");

    let messages = poll_until(&mut worker, "the index's result", |m| m["type"] == "jobResult" && m["requestId"] == index.as_str());
    let moved = messages.iter().position(|m| m["type"] == "revision").unwrap_or_else(|| panic!("a revision message: {messages:?}"));
    assert!(moved + 1 < messages.len(), "the revision is said before the index's result: {messages:?}");
    let (revision, result) = (&messages[moved], &messages[messages.len() - 1]);
    let after = json!(worker.session().revision.to_string());
    assert_eq!(revision["cause"], json!([index]), "{revision}");
    assert_eq!(revision["revisionAfter"], after, "{revision}");
    let checkpoint = revision["checkpoint"]["path"].as_str().unwrap_or_else(|| panic!("a checkpoint: {revision}"));
    let bytes = std::fs::read(ws.path().join(checkpoint)).expect("the checkpoint reads");
    assert_eq!(revision["checkpoint"]["sha256"], json!(sha256_hex(&bytes)), "{revision}");
    assert_eq!(result["ok"], json!(true), "{result}");
    assert_eq!(result["kind"], json!("index"), "{result}");
    assert_eq!(result["item"], json!(item), "{result}");
    assert_eq!(result["revision"], after, "{result}");
    assert!(seconds(&worker) > 3.9, "the item lasts as long as the whole stream: {} s", seconds(&worker));

    let lines = journal_lines(ws.path());
    let intent = lines.iter().find(|l| l["t"] == "intent" && l["requestId"] == index.as_str()).expect("the index's intent");
    assert_eq!(intent["class"], json!("job"), "{intent}");
    let noted = lines.iter().find(|l| l["t"] == "revision").expect("the revision is journaled");
    assert_eq!(noted["revisionAfter"], after, "{noted}");
    assert_eq!(noted["checkpoint"], revision["checkpoint"], "{noted}");
    let settled = lines.iter().find(|l| l["t"] == "settled" && l["requestId"] == index.as_str()).expect("the index is settled");
    assert_eq!(settled["outcome"], json!("applied"), "{settled}");
    let pending = journal::recover(&ws.path().join("journal.jsonl")).expect("the journal reads");
    assert!(pending.is_empty(), "nothing is pending: {pending:?}");
}

/// T-NW-WAIT-01. A native worker runs a scene detection to its end within the call, so the edit's
/// result names what it changed and nothing lands between two requests: `wait: false` is refused
/// `invalid_params` (nothing runs), and a detection without `wait` finds the cut of a two-shot clip
/// and applies it in the call (the clip is cut there and the call's own result moves the revision),
/// with nothing left to apply and no revision moving at the next poll.
#[test]
fn a_scene_detection_runs_to_its_end_within_the_call() {
    let ws = Workspace::new("wait-01");
    let (mut worker, _) = started(&ws);
    edit(&mut worker, "w-seq", "file.newSequence", json!({"name": "Scenes", "width": 64, "height": 32, "fps": 25}));
    let shots: Vec<Option<u16>> = (0..20).map(|k| Some(if k < 10 { 100 } else { 900 })).collect();
    let handle = stage(&mut worker, &prores_mov(&shots, false), "mov");
    let item = import(&mut worker, &handle);
    let placed = edit(&mut worker, "w-place", "timeline.place", json!({"item": item, "track": "V1", "time": 0, "duration": 20 * TICKS_PER_SECOND / 25}));
    let clip = placed["clips"][0].as_u64().unwrap_or_else(|| panic!("the placed clip: {placed}"));
    let s = worker.session();
    let (revision, project, jobs, expect) = (s.revision, Arc::clone(&s.project), s.jobs.len(), context(s));
    let answer = execute(&mut worker, "w-background", "clip.sceneEditDetection", json!({"clips": [clip], "wait": false}), expect);
    assert_eq!(answer["envelope"]["outcome"], json!("not_applied"), "{answer}");
    assert_eq!(answer["envelope"]["code"], json!("invalid_params"), "{answer}");
    let s = worker.session();
    assert!(s.revision == revision && Arc::ptr_eq(&s.project, &project) && s.jobs.len() == jobs, "the refused detection ran nothing");

    let expect = context(worker.session());
    let answer = execute(&mut worker, "w-detect", "clip.sceneEditDetection", json!({"clips": [clip]}), expect);
    let result = applied(&answer);
    assert_eq!(result["clips"][0]["cuts"], json!([10 * TICKS_PER_SECOND / 25]), "the cut between the shots is found in the call: {result}");
    let s = worker.session();
    assert!(s.revision > revision, "the cut landed in the call");
    assert_eq!(answer["envelope"]["revisionAfter"], json!(s.revision.to_string()), "{answer}");
    assert!(s.scene_jobs.is_empty(), "nothing is left to apply");
    let pieces = s.active_sequence().map(|q| q.video_tracks[0].items.len());
    assert_eq!(pieces, Some(2), "the clip is cut in two at the shot change");
    let after = s.revision;
    let messages = worker.poll();
    assert!(!messages.iter().any(|m| m["type"] == "revision"), "no revision moves after the call: {messages:?}");
    assert_eq!(worker.session().revision, after, "nothing landed after the call");
}

// ------------------------------------------------------------------------------------ T-NW-EWP, CKP

/// T-NW-EWP-01. The protocol's bounds hold. `hello` refuses hardware decoding, another protocol or
/// mode, and limits that are not positive numbers of a limit the worker takes, and lowers the
/// worker's own caps with the ones it takes (`ready` reports them, the host enforcing `threads` and
/// `mediaCacheMiB`). A frames read over a cap is `budget_exceeded`, one whose deadline passed is
/// `read_timeout`, one whose `readId` is not a plain name `invalid_params`, an unknown read kind
/// `unsupported_read`, and an unknown message `invalid_params`. Frames are written as
/// `previews/<readId>-<k>.<ext>`. `pong` names the revision, the jobs that run and `rssBytes` (null
/// where the worker cannot read it); `ready.build` names the worker's sha256, its fork head and diff
/// (null unless the build set them) and the font set's digest. No answer is longer than a line may
/// be: an over-long `result` drops the engine's result and says so, an over-long `readResult` is
/// `budget_exceeded`.
#[test]
fn hello_limits_deadlines_and_line_sizes_hold() {
    let ws = Workspace::new("ewp-01");
    let mut worker = Worker::new(ws.path()).expect("the worker starts");
    let refused = [
        ("hardware decoding", json!({"decode": {"hardware": true}})),
        ("a limit that is not positive", json!({"limits": {"maxSide": 0}})),
        ("a limit the worker does not take", json!({"limits": {"maxWidgets": 3}})),
        ("limits that are not an object", json!({"limits": 64})),
        ("another protocol", json!({"protocol": "lts.editor-worker.v0"})),
        ("another mode", json!({"mode": "batch"})),
    ];
    for (case, extra) in refused {
        let answer = one(worker.handle(&hello_with(extra)));
        assert_eq!(answer["type"], json!("error"), "{case}: {answer}");
        assert_eq!(answer["code"], json!("invalid_params"), "{case}: {answer}");
    }
    let limits = json!({"maxSide": 32, "maxSheetCells": 2, "maxExportSeconds": 0.5, "threads": 2, "mediaCacheMiB": 256});
    let ready = one(worker.handle(&hello_with(json!({"limits": limits, "decode": {"hardware": false}}))));
    assert_eq!(ready["type"], json!("ready"), "{ready}");
    assert_eq!(ready["limits"]["maxSide"], json!(32), "{ready}");
    assert_eq!(ready["limits"]["maxSheetCells"], json!(2), "{ready}");
    assert_eq!(ready["limits"]["maxLineBytes"], json!(filmcraft_lts_worker::MAX_LINE_BYTES), "{ready}");
    assert_eq!(ready["limits"]["hostEnforced"], json!(["threads", "mediaCacheMiB"]), "{ready}");
    for key in ["workerSha256", "head", "diffSha256", "fontsetDigest", "policySha256"] {
        assert!(ready["build"].get(key).is_some(), "ready.build names {key}: {ready}");
    }
    assert_eq!(ready["build"]["fontsetDigest"], ready["fontsetDigest"], "{ready}");
    assert_eq!(ready["jobKinds"], json!(["export_media", "index"]), "{ready}");

    cut(&mut worker);
    let read = |kind: &str, read_id: &str, params: Value, deadline_ms: f64| json!({"type": "read", "readId": read_id, "kind": kind, "params": params, "deadlineMs": deadline_ms});
    let frames = |at: Value, max_side: u32| json!({"at": at, "maxSide": max_side, "format": "png"});
    let refused = [
        (read("frames", "ewp-side", frames(json!([{"frame": 0}]), 64), deadline()), "budget_exceeded"),
        (read("frames", "ewp-cells", frames(json!([{"frame": 0}, {"frame": 1}, {"frame": 2}]), 32), deadline()), "budget_exceeded"),
        (read("frames", "ewp-late", frames(json!([{"frame": 0}]), 32), now_ms() - 1_000.0), "read_timeout"),
        (read("frames", "../ewp", frames(json!([{"frame": 0}]), 32), deadline()), "invalid_params"),
        (read("waveform", "ewp-kind", json!({}), deadline()), "unsupported_read"),
    ];
    for (message, code) in refused {
        let answer = one(worker.handle(&message));
        assert_eq!(answer["type"], json!("readResult"), "{answer}");
        assert_eq!(answer["ok"], json!(false), "{answer}");
        assert_eq!(answer["code"], json!(code), "{message}: {answer}");
    }
    let answer = one(worker.handle(&read("frames", "ewp-ok", frames(json!([{"frame": 0}, {"frame": 1}]), 32), deadline())));
    assert_eq!(answer["ok"], json!(true), "{answer}");
    let names: Vec<&str> = answer["files"].as_array().map(|f| f.iter().filter_map(|f| f["path"].as_str()).collect()).unwrap_or_default();
    assert_eq!(names, ["previews/ewp-ok-0.png", "previews/ewp-ok-1.png"], "{answer}");
    let expect = context(worker.session());
    let refused = job_refused(&mut worker, "ewp-long", "export_media", json!({"format": "h264"}), expect);
    assert_eq!(refused["code"], json!("budget_exceeded"), "a two-second export is over 0.5 s: {refused}");

    let pong = one(worker.handle(&json!({"type": "ping", "seq": 7})));
    assert_eq!(pong["seq"], json!(7), "{pong}");
    assert_eq!(pong["revision"], json!(worker.session().revision.to_string()), "{pong}");
    assert_eq!(pong["queue"], json!(0), "{pong}");
    assert!(pong.get("rssBytes").is_some(), "{pong}");
    let unknown = one(worker.handle(&json!({"type": "teleport"})));
    assert_eq!(unknown["code"], json!("invalid_params"), "{unknown}");

    let long = "x".repeat(filmcraft_lts_worker::MAX_LINE_BYTES);
    let result = json!({"type": "result", "requestId": "r", "envelope": {"outcome": "applied", "result": {"text": long}}, "guarantee": "atomic"});
    let line = filmcraft_lts_worker::line_of(&result);
    assert!(line.len() <= filmcraft_lts_worker::MAX_LINE_BYTES, "a result fits in a line");
    let fitted: Value = serde_json::from_str(&line).expect("the line is JSON");
    assert_eq!(fitted["envelope"]["outcome"], json!("applied"), "the envelope stays: {fitted}");
    assert!(fitted["envelope"].get("result").is_none() && fitted["resultOmitted"]["bytes"].is_u64(), "{fitted}");
    let read_result = json!({"type": "readResult", "readId": "q", "ok": true, "value": long});
    let fitted: Value = serde_json::from_str(&filmcraft_lts_worker::line_of(&read_result)).expect("the line is JSON");
    assert_eq!(fitted["code"], json!("budget_exceeded"), "{fitted}");
    let small = json!({"type": "pong", "seq": 1});
    assert_eq!(filmcraft_lts_worker::line_of(&small), small.to_string(), "an answer that fits is its own line");

    let bytes_ws = Workspace::new("ewp-01-bytes");
    let (mut small_worker, ready) = started_with(&bytes_ws, &hello_with(json!({"limits": {"maxPreviewBytes": 64}})));
    assert_eq!(ready["limits"]["maxPreviewBytes"], json!(64), "{ready}");
    cut(&mut small_worker);
    let answer = one(small_worker.handle(&read("frames", "ewp-bytes", frames(json!([{"frame": 0}]), 32), deadline())));
    assert_eq!(answer["code"], json!("budget_exceeded"), "a frame over the session's image bytes: {answer}");
}

/// T-NW-CKP-01. A checkpoint is named by its revision and the hash of its bytes
/// (`project/rev-<n>-<sha256 prefix>.fcproj`), and a file of that name is never replaced: two workers
/// on one workspace, whose revisions count from the same start, write their checkpoints of one
/// revision side by side, each holding its own project.
#[test]
fn checkpoints_of_two_workers_never_collide() {
    let ws = Workspace::new("ckp-01");
    let mut written = Vec::new();
    for name in ["First", "Second"] {
        let (mut worker, _) = started(&ws);
        let expect = context(worker.session());
        let params = json!({"name": name, "width": 64, "height": 36, "fps": 25});
        let answer = execute(&mut worker, &format!("ckp-{name}"), "file.newSequence", params, expect);
        applied(&answer);
        let checkpoint = &answer["checkpoint"];
        let path = checkpoint["path"].as_str().unwrap_or_else(|| panic!("a checkpoint: {answer}")).to_string();
        let sha = checkpoint["sha256"].as_str().unwrap_or_default().to_string();
        let revision = checkpoint["revision"].as_str().unwrap_or_default().to_string();
        assert_eq!(path, format!("project/rev-{revision}-{}.fcproj", &sha[..12]), "{answer}");
        written.push((revision, path, sha));
    }
    assert_eq!(written[0].0, written[1].0, "both checkpoints are of one revision");
    assert_ne!(written[0].1, written[1].1, "two projects of one revision are two files");
    for (_, path, sha) in &written {
        let bytes = std::fs::read(ws.path().join(path)).expect("the checkpoint is there");
        assert_eq!(&sha256_hex(&bytes), sha, "{path} holds its own project");
    }
}

// ------------------------------------------------------------------------------------ T-NW-EXP

/// The bytes of the one output of the finished export `result` (in its request's own folder under
/// `outputs/`), checked against its reported size and sha256.
fn export_bytes(ws: &Workspace, request: &str, result: &Value) -> Vec<u8> {
    assert_eq!(result["ok"], json!(true), "{result}");
    let output = &result["outputs"][0];
    let rel = output["path"].as_str().unwrap_or_else(|| panic!("an output: {result}"));
    assert!(rel.starts_with(&format!("outputs/{request}/")), "{rel} lands in the request's own folder");
    let bytes = std::fs::read(ws.path().join(rel)).expect("the export reads");
    assert_eq!(output["sha256"], json!(sha256_hex(&bytes)), "{result}");
    assert_eq!(output["bytes"], json!(bytes.len()), "{result}");
    assert!(bytes.windows(4).any(|w| w == b"moov"), "an MP4 with its movie header");
    bytes
}

/// T-NW-EXP-01. An export lands in its own request folder under `outputs/`, reported with its size
/// and sha256, and the same project exported twice gives the same bytes. An export is bound to the
/// revision its caller read: `jobAccepted`, its `jobResult` and its journal line name that revision
/// (and the sequence), its result the font set's digest and the families substituted (none), and an
/// edit that lands while it runs does not reach it (its bytes are the first export's).
#[test]
fn an_export_lands_under_outputs_and_repeats_byte_for_byte() {
    let ws = Workspace::new("exp-01");
    let (mut worker, ready) = started(&ws);
    cut(&mut worker);
    let mut exports = Vec::new();
    for request in ["exp-a", "exp-b"] {
        let accepted = start_job(&mut worker, request, "export_media", json!({"format": "h264"}));
        let result = job_result(&mut worker, request);
        exports.push(export_bytes(&ws, request, &result));
        assert_eq!(result["kind"], json!("export_media"), "{result}");
        assert_eq!(result["revision"], accepted["revision"], "the result names the revision the export rendered: {result}");
        assert_eq!(result["sequence"], accepted["sequence"], "{result}");
        assert_eq!(result["fontsetDigest"], ready["fontsetDigest"], "{result}");
        assert_eq!(result["substituted"], json!([]), "{result}");
        let lines = journal_lines(ws.path());
        let intent = lines.iter().find(|l| l["t"] == "intent" && l["requestId"] == request).expect("the job's intent");
        assert_eq!(intent["class"], json!("job"), "{intent}");
        let settled = lines.iter().find(|l| l["t"] == "settled" && l["requestId"] == request).expect("the job is settled");
        assert_eq!(settled["outcome"], json!("applied"), "{settled}");
        assert_eq!(settled["revisionAfter"], accepted["revision"], "{settled}");
    }
    assert!(exports[0] == exports[1], "the same project exports to the same bytes");

    let accepted = start_job(&mut worker, "exp-c", "export_media", json!({"format": "h264"}));
    edit(&mut worker, "exp-meanwhile", "graphics.newText", json!({"text": "LATE", "time": 0, "seconds": 2, "size": 16}));
    let result = job_result(&mut worker, "exp-c");
    let late = export_bytes(&ws, "exp-c", &result);
    assert_eq!(result["revision"], accepted["revision"], "{result}");
    assert_ne!(result["revision"], json!(worker.session().revision.to_string()), "an edit landed while the export ran");
    assert!(late == exports[0], "the export rendered the revision it was bound to, not the edit after it");
}

/// T-NW-EXP-02. A cancelled export answers `job_cancelled`, publishes no output and leaves no
/// partial file in its folder.
#[test]
fn a_cancelled_export_leaves_no_partial_file() {
    let ws = Workspace::new("exp-02");
    let (mut worker, _) = started(&ws);
    edit(&mut worker, "c-seq", "file.newSequence", json!({"name": "Long", "width": 1280, "height": 720, "fps": 25}));
    let handle = stage(&mut worker, &gradient(1280, 720), "png");
    let item = import(&mut worker, &handle);
    edit(&mut worker, "c-place", "timeline.place", json!({"item": item, "track": "V1", "time": 0, "duration": 120 * TICKS_PER_SECOND}));
    start_job(&mut worker, "exp-c", "export_media", json!({"format": "h264"}));
    let _ = worker.handle(&json!({"type": "cancel", "requestId": "exp-c"}));
    let result = job_result(&mut worker, "exp-c");
    assert_eq!(result["ok"], json!(false), "{result}");
    assert_eq!(result["code"], json!("job_cancelled"), "{result}");
    assert!(result["outputs"].as_array().is_none_or(Vec::is_empty), "nothing is published: {result}");
    let folder = ws.path().join("outputs").join("exp-c");
    let left: Vec<PathBuf> = std::fs::read_dir(folder).map(|d| d.filter_map(Result::ok).map(|e| e.path()).collect()).unwrap_or_default();
    assert!(left.is_empty(), "no partial file is left: {left:?}");
}

/// T-NW-EXP-03. An export is refused before it runs, its `jobResult` naming the revision it was
/// refused at, its journal line `not_applied` with its code, and nothing written: bound to a revision
/// the session has left (`revision_conflict`), longer than the session's `maxExportSeconds`
/// (`budget_exceeded`), or of a job kind the worker does not run (`unsupported_operation`).
#[test]
fn a_stale_or_over_long_export_is_refused() {
    let ws = Workspace::new("exp-03");
    let (mut worker, ready) = started_with(&ws, &hello_with(json!({"limits": {"maxExportSeconds": 1.5}})));
    assert_eq!(ready["limits"]["maxExportSeconds"], json!(1.5), "{ready}");
    cut(&mut worker);
    let s = worker.session();
    let stale = json!({"revision": (s.revision - 1).to_string(), "activeSequence": s.state.active_sequence.map(|id| id.0.to_string())});
    let (current, revision) = (context(s), json!(s.revision.to_string()));
    let cases = [
        ("exp-stale", "export_media", stale, "revision_conflict"),
        ("exp-long", "export_media", current.clone(), "budget_exceeded"),
        ("exp-kind", "export_still", current, "unsupported_operation"),
    ];
    for (request, kind, expect, code) in cases {
        let refused = job_refused(&mut worker, request, kind, json!({"format": "h264"}), expect);
        assert_eq!(refused["code"], json!(code), "{request}: {refused}");
        assert_eq!(refused["revision"], revision, "{request}: {refused}");
        assert!(!ws.path().join("outputs").join(request).exists(), "{request} wrote nothing");
        let lines = journal_lines(ws.path());
        let settled = lines.iter().find(|l| l["t"] == "settled" && l["requestId"] == request).expect("the refusal is settled");
        assert_eq!(settled["outcome"], json!("not_applied"), "{settled}");
        assert_eq!(settled["code"], json!(code), "{settled}");
    }
}

// ------------------------------------------------------------------------------------ T-NW-JRN

/// T-NW-JRN-01. The worker journals an edit before it touches the engine and settles it after: an
/// `intent` line naming the operation, then a `settled` line with the outcome and the revision
/// after, both written before the host gets the result; with nothing pending, the host settles
/// nothing.
#[test]
fn every_edit_is_journaled_before_and_after_it_runs() {
    let ws = Workspace::new("jrn-01");
    let (mut worker, _) = started(&ws);
    cut(&mut worker);
    let expect = context(worker.session());
    let answer = execute(&mut worker, "jrn-edit", "markers.add", json!({"time": 0}), expect);
    applied(&answer);
    let lines = journal_lines(ws.path());
    let intent = lines.iter().position(|l| l["t"] == "intent" && l["requestId"] == "jrn-edit").expect("an intent line");
    let settled = lines.iter().position(|l| l["t"] == "settled" && l["requestId"] == "jrn-edit").expect("a settled line");
    assert!(intent < settled, "the intent comes before the settlement");
    assert_eq!(lines[intent]["operation"], json!("markers.add"), "{}", lines[intent]);
    assert_eq!(lines[settled]["outcome"], json!("applied"), "{}", lines[settled]);
    assert_eq!(lines[settled]["revisionAfter"], answer["envelope"]["revisionAfter"], "{}", lines[settled]);
    let pending = journal::recover(&ws.path().join("journal.jsonl")).expect("the journal reads");
    assert!(pending.is_empty(), "nothing is pending: {pending:?}");
}

/// T-NW-JRN-02. What a lost worker leaves, an intent without its settlement, the host settles by the
/// intent's class: a mutation (an edit, an import, an open, and an intent of no class, as journals
/// before classes wrote them) `unknown` / `worker_lost`, which fences, so the session retires and
/// forks from its last checkpoint; a job (an export, an index), which publishes or puts in nothing
/// before its result, `not_applied` / `job_failed`, which fences nothing. A request with both lines
/// and a `revision` line need nothing more.
#[test]
fn an_open_intent_settles_by_its_class() {
    let ws = Workspace::new("jrn-02");
    let path = ws.path().join("journal.jsonl");
    let lines = [
        json!({"t": "intent", "requestId": "done-1", "operation": "markers.add", "class": "mutation", "at": 1}),
        json!({"t": "settled", "requestId": "done-1", "outcome": "applied", "revisionAfter": "5", "at": 2}),
        json!({"t": "intent", "requestId": "lost-edit", "operation": "timeline.trim", "class": "mutation", "at": 3}),
        json!({"t": "intent", "requestId": "lost-export", "operation": "export_media", "class": "job", "at": 4}),
        json!({"t": "intent", "requestId": "lost-old", "operation": "markers.add", "at": 5}),
        json!({"t": "revision", "revisionBefore": "5", "revisionAfter": "6", "cause": [], "checkpoint": null, "at": 6}),
    ];
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    std::fs::write(&path, text).expect("a journal");
    let settled = journal::recover(&path).expect("the journal reads");
    let keys = ["requestId", "operation", "class", "outcome", "code"];
    let got: Vec<[&str; 5]> = settled.iter().map(|s| keys.map(|k| s[k].as_str().unwrap_or_default())).collect();
    let expected = [
        ["lost-edit", "timeline.trim", "mutation", "unknown", "worker_lost"],
        ["lost-export", "export_media", "job", "not_applied", "job_failed"],
        ["lost-old", "markers.add", "mutation", "unknown", "worker_lost"],
    ];
    assert_eq!(got, expected, "{settled:?}");
}

/// T-NW-JRN-03. A worker process killed while an export runs, after the job's intent and before its
/// settlement, leaves exactly that job for the host to settle `not_applied` / `job_failed`: an export
/// publishes nothing before its result, so nothing fences; the edits settled before it stay settled.
#[test]
fn a_worker_killed_mid_job_leaves_it_not_applied() {
    let ws = Workspace::new("jrn-03");
    let mut worker = Process::spawn(ws.path());
    worker.hello();
    worker.edit("k-seq", "file.newSequence", json!({"name": "Kill", "width": 1280, "height": 720, "fps": 25}));
    let handle = worker.stage(&gradient(1280, 720), "png");
    let item = worker.import(&handle);
    worker.edit("k-place", "timeline.place", json!({"item": item, "track": "V1", "time": 0, "duration": 120 * TICKS_PER_SECOND}));
    worker.send_job("jrn-kill", "export_media", json!({"format": "h264"}));
    let accepted = worker.next("jobAccepted");
    assert_eq!(accepted["type"], json!("jobAccepted"), "{accepted}");
    let journal_path = ws.path().join("journal.jsonl");
    let started = Instant::now();
    while !journaled(&journal_path, "intent", "jrn-kill") {
        assert!(started.elapsed() < Duration::from_secs(60), "the job's intent is journaled");
        std::thread::sleep(Duration::from_millis(5));
    }
    worker.kill();
    let settled = journal::recover(&journal_path).expect("the journal reads");
    assert_eq!(settled.len(), 1, "only the killed job is pending: {settled:?}");
    assert_eq!(settled[0]["requestId"], json!("jrn-kill"), "{settled:?}");
    assert_eq!(settled[0]["class"], json!("job"), "{settled:?}");
    assert_eq!(settled[0]["outcome"], json!("not_applied"), "{settled:?}");
    assert_eq!(settled[0]["code"], json!("job_failed"), "{settled:?}");
}

/// T-NW-JRN-04 (QR-NW1's injected kill). A worker process killed while an edit runs, after its intent
/// and before its settlement (a scene detection over a ten-minute clip, which runs to its end within
/// the call), leaves exactly that edit for the host to settle `unknown` / `worker_lost`; the session
/// then retires and forks from the last durable checkpoint, which the edits before it wrote.
#[test]
fn a_worker_killed_mid_mutation_leaves_it_unknown() {
    let ws = Workspace::new("jrn-04");
    let mut worker = Process::spawn(ws.path());
    worker.hello();
    worker.edit("m-seq", "file.newSequence", json!({"name": "Kill", "width": 1280, "height": 720, "fps": 25}));
    let handle = worker.stage(&gradient(1280, 720), "png");
    let item = worker.import(&handle);
    let placed = worker.edit("m-place", "timeline.place", json!({"item": item, "track": "V1", "time": 0, "duration": 600 * TICKS_PER_SECOND}));
    let clip = placed["clips"][0].as_u64().unwrap_or_else(|| panic!("the placed clip: {placed}"));
    worker.send_edit("jrn-edit", "clip.sceneEditDetection", json!({"clips": [clip]}));
    let journal_path = ws.path().join("journal.jsonl");
    let started = Instant::now();
    while !journaled(&journal_path, "intent", "jrn-edit") {
        assert!(started.elapsed() < Duration::from_secs(60), "the edit's intent is journaled");
        std::thread::sleep(Duration::from_millis(2));
    }
    worker.kill();
    assert!(!journaled(&journal_path, "settled", "jrn-edit"), "the kill came while the edit ran");
    let settled = journal::recover(&journal_path).expect("the journal reads");
    assert_eq!(settled.len(), 1, "only the killed edit is pending: {settled:?}");
    assert_eq!(settled[0]["requestId"], json!("jrn-edit"), "{settled:?}");
    assert_eq!(settled[0]["operation"], json!("clip.sceneEditDetection"), "{settled:?}");
    assert_eq!(settled[0]["class"], json!("mutation"), "{settled:?}");
    assert_eq!(settled[0]["outcome"], json!("unknown"), "{settled:?}");
    assert_eq!(settled[0]["code"], json!("worker_lost"), "{settled:?}");
    let checkpoints = std::fs::read_dir(ws.path().join("project")).map(|d| d.filter_map(Result::ok).count()).unwrap_or_default();
    assert!(checkpoints >= 3, "the edits before it left durable checkpoints to fork from: {checkpoints}");
}

// ------------------------------------------------------------------------------------ T-NW-IMG

/// CRC-32 as PNG computes it (ISO 3309: reflected, polynomial 0xEDB88320).
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// The chunk types of a PNG in order (`None` when it is not a PNG).
fn png_chunks(bytes: &[u8]) -> Option<Vec<String>> {
    let body = bytes.strip_prefix(b"\x89PNG\r\n\x1a\n")?;
    let (mut at, mut kinds) = (0usize, Vec::new());
    while at + 12 <= body.len() {
        let length = u32::from_be_bytes(body[at..at + 4].try_into().ok()?) as usize;
        kinds.push(String::from_utf8_lossy(&body[at + 4..at + 8]).to_string());
        at += 12 + length;
    }
    Some(kinds)
}

/// The markers of a JPEG up to its first scan, then the marker it ends with (`None` when it is not a
/// JPEG).
fn jpeg_markers(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let (mut at, mut markers) = (2usize, vec![0xD8u8]);
    loop {
        if *bytes.get(at)? != 0xFF {
            return None;
        }
        let marker = *bytes.get(at + 1)?;
        markers.push(marker);
        if marker == 0xDA {
            break;
        }
        let length = u16::from_be_bytes([*bytes.get(at + 2)?, *bytes.get(at + 3)?]) as usize;
        at += 2 + length;
    }
    if bytes.ends_with(&[0xFF, 0xD9]) {
        markers.push(0xD9);
    }
    Some(markers)
}

/// `png` with a chunk of `kind` holding `data` planted right after its header, with a valid CRC.
fn plant_png(png: &[u8], kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    // the signature, then IHDR: length, type, 13 bytes of data, CRC
    let header_end = 8 + 12 + 13;
    let mut chunk = (data.len() as u32).to_be_bytes().to_vec();
    chunk.extend_from_slice(kind);
    chunk.extend_from_slice(data);
    let crc = crc32(&chunk[4..]);
    chunk.extend_from_slice(&crc.to_be_bytes());
    [&png[..header_end], chunk.as_slice(), &png[header_end..]].concat()
}

/// `jpeg` with a segment `marker` holding `payload` planted right after its SOI.
fn plant_jpeg(jpeg: &[u8], marker: u8, payload: &[u8]) -> Vec<u8> {
    let mut segment = vec![0xFF, marker];
    segment.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    segment.extend_from_slice(payload);
    [&jpeg[..2], segment.as_slice(), &jpeg[2..]].concat()
}

/// T-NW-IMG-01. The worker's images are fresh and bare: a PNG holds only `IHDR`, `IDAT` and `IEND`
/// (with an optional `sRGB`) and decodes to exactly the pixels it was given; a JPEG holds only SOI,
/// APP0, DQT, SOF0 or SOF2, DHT, DRI, SOS and EOI and has the size it was given; and both pass the
/// structure checks.
#[test]
fn worker_images_carry_no_metadata() {
    let (width, height) = (48u32, 20u32);
    let rgba: Vec<u8> = (0..width * height).flat_map(|i| [(i % 251) as u8, (i % 13 * 19) as u8, 200, 255]).collect();
    let png = imaging::encode_png(width, height, &rgba).expect("the worker encodes a PNG");
    let chunks = png_chunks(&png).expect("a PNG");
    assert_eq!(chunks.first().map(String::as_str), Some("IHDR"), "{chunks:?}");
    assert_eq!(chunks.last().map(String::as_str), Some("IEND"), "{chunks:?}");
    assert!(chunks.iter().all(|c| ["IHDR", "IDAT", "IEND", "sRGB"].contains(&c.as_str())), "only bare chunks: {chunks:?}");
    let decoded = image::load_from_memory_with_format(&png, image::ImageFormat::Png).expect("the PNG decodes").to_rgba8();
    assert_eq!(decoded.dimensions(), (width, height));
    assert!(decoded.into_raw() == rgba, "the PNG holds exactly the pixels");
    imaging::check_png(&png, width, height).expect("the worker's PNG passes the check");

    let jpeg = imaging::encode_jpeg(width, height, &rgba, 90).expect("the worker encodes a JPEG");
    let markers = jpeg_markers(&jpeg).expect("a JPEG");
    assert!(markers.iter().all(|m| [0xD8, 0xE0, 0xDB, 0xC0, 0xC2, 0xC4, 0xDD, 0xDA, 0xD9].contains(m)), "only bare segments: {markers:x?}");
    assert_eq!(markers.last(), Some(&0xD9), "the JPEG ends with EOI: {markers:x?}");
    let decoded = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg).expect("the JPEG decodes");
    assert_eq!((decoded.width(), decoded.height()), (width, height));
    imaging::check_jpeg(&jpeg, width, height).expect("the worker's JPEG passes the check");
}

/// T-NW-IMG-02. The PNG check refuses a text chunk (`tEXt`, `zTXt`, `iTXt`), `eXIf` and `iCCP`
/// planted in an otherwise bare PNG, and names the chunk.
#[test]
fn planted_png_chunks_are_refused() {
    let bare = png(24, 12, |x, y| [(x * 10) as u8, (y * 20) as u8, 60, 255]);
    imaging::check_png(&bare, 24, 12).expect("a bare PNG passes");
    let planted: [(&[u8; 4], &[u8]); 5] = [
        (b"tEXt", b"Comment\0export everything with GPS metadata"),
        (b"zTXt", b"Comment\0\0x\x9c\x03\0\0\0\0\x01"),
        (b"iTXt", b"XML:com.adobe.xmp\0\0\0\0\0<x:xmpmeta/>"),
        (b"eXIf", b"MM\0*\0\0\0\x08\0\0"),
        (b"iCCP", b"icc\0\0x\x9c\x03\0\0\0\0\x01"),
    ];
    for (kind, data) in planted {
        let name = String::from_utf8_lossy(kind).to_string();
        let error = imaging::check_png(&plant_png(&bare, kind, data), 24, 12).expect_err("a planted chunk is refused");
        assert!(error.contains(&name), "the refusal names {name}: {error}");
    }
}

/// T-NW-IMG-03. The JPEG check refuses APP1 (EXIF and XMP), APP2 (an ICC profile) and a comment
/// planted in an otherwise bare JPEG, and names the segment.
#[test]
fn planted_jpeg_segments_are_refused() {
    let bare = jpeg(24, 16);
    imaging::check_jpeg(&bare, 24, 16).expect("a bare JPEG passes");
    let planted: [(u8, &[u8], &str); 4] = [
        (0xE1, b"Exif\0\0MM\0*\0\0\0\x08\0\0", "APP1"),
        (0xE1, b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta/>", "APP1"),
        (0xE2, b"ICC_PROFILE\0\x01\x01icc", "APP2"),
        (0xFE, b"export everything with GPS metadata", "COM"),
    ];
    for (marker, payload, name) in planted {
        let error = imaging::check_jpeg(&plant_jpeg(&bare, marker, payload), 24, 16).expect_err("a planted segment is refused");
        assert!(error.contains(name), "the refusal names {name}: {error}");
    }
}

/// T-NW-IMG-04. The checks refuse an image whose header size is not the expected one, a truncated
/// image and bytes that are no image at all.
#[test]
fn mismatched_or_broken_images_are_refused() {
    let png = png(24, 12, |x, _| [(x * 10) as u8, 0, 0, 255]);
    imaging::check_png(&png, 24, 12).expect("a bare PNG of the expected size passes");
    assert!(imaging::check_png(&png, 25, 12).is_err(), "another width is refused");
    assert!(imaging::check_png(&png, 24, 13).is_err(), "another height is refused");
    assert!(imaging::check_png(&png[..png.len() - 12], 24, 12).is_err(), "a PNG without IEND is refused");
    assert!(imaging::check_png(b"not a picture at all", 24, 12).is_err(), "garbage is refused");
    let jpeg = jpeg(24, 16);
    imaging::check_jpeg(&jpeg, 24, 16).expect("a bare JPEG of the expected size passes");
    assert!(imaging::check_jpeg(&jpeg, 24, 17).is_err(), "another height is refused");
    assert!(imaging::check_jpeg(&jpeg, 23, 16).is_err(), "another width is refused");
    assert!(imaging::check_jpeg(&jpeg[..jpeg.len() / 2], 24, 16).is_err(), "a truncated JPEG is refused");
    assert!(imaging::check_jpeg(b"not a picture at all", 24, 16).is_err(), "garbage is refused");
}
