//! T-NW-SMOKE-BB5: the scripted smoke run of the LTS native worker on Big Brother material (slice
//! NW-1a; master plan v3 W7; native-worker-architecture.md §8.4, "Scripted smoke on BB"). No model:
//! the test drives the worker over EWP in this process as the host does, on BB5, the 306 s chunk
//! staged as `lts-bbf7a4aa….mp4` (1280×720 H.264 at 25 fps, AAC 48 kHz stereo starting at 0.020 s,
//! colour untagged): stage, place 300 frames from source frame 1000 on a 1080×1920 sequence, fill
//! the frame (Motion), read a sheet, export, decode. ffprobe and ffmpeg check the export as external
//! oracles (never linked):
//!
//! - it holds 300 frames at 25/1 with consecutive PTS;
//! - each decoded frame k matches the project render at k above a PSNR floor, and the sequence maps
//!   frame k to source frame 1000 + k (a crop cannot be compared with the source pixel for pixel,
//!   so identity runs through the project render);
//! - its picture and sound durations are reported;
//! - the measured `audioStart` (the export's sound against the source's at the same PTS, by
//!   cross-correlation) and the colour interpretation (the export's tags, and whether the engine
//!   reads the untagged source as limited or full range) are recorded.
//!
//! The report is printed and written beside the workspace (`bb5-smoke-report.json`), before the
//! checks fail if one does.
//!
//! Ignored by default: it needs BB5, which only the LTS harness holds (OC-1: Big Brother media stay
//! on this machine), and ffmpeg. The workspace is `<CARGO_TARGET_TMPDIR>/nw1a-bb5-smoke`; BB5 is
//! hard-linked into its `files/` (never copied) and the link stays there between runs: BB5 is
//! read-only, and removing a read-only link would change the original's attributes. Run it with
//! `cargo test -p filmcraft-lts-worker --test bb5_smoke -- --ignored --nocapture`.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use filmcraft_engine::Session;
use filmcraft_engine::project::{ClipId, ItemId};
use filmcraft_engine::time::{TICKS_PER_SECOND, Tick};
use filmcraft_lts_worker::Worker;
use serde_json::{Value, json};

/// BB5 by its sha256 and size (master plan v3, the fixture table).
const BB5_SHA256: &str = "bbf7a4aa5f32769ea1563242b39b5a5a71b0899e42dcc5e0fce1aa36044f4831";
const BB5_BYTES: u64 = 42_057_450;

/// The source frame the placed range starts at, and its length in frames.
const SOURCE_IN: i64 = 1000;
const FRAMES: i64 = 300;

/// The PSNR every export frame keeps against the project render, in dB.
const PSNR_FLOOR: f64 = 30.0;

/// The rate the sound is compared at, and how far around the source's PTS the lag is searched (one
/// frame each way).
const RATE: usize = 48_000;
const LAG_SPAN: usize = RATE / 25;

/// Milliseconds since the Unix epoch, the clock of an EWP deadline.
fn now_ms() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("the clock is past 1970").as_millis() as f64
}

/// A deadline ten minutes ahead.
fn deadline() -> f64 {
    now_ms() + 600_000.0
}

/// The one message `messages` holds.
fn one(messages: Vec<Value>) -> Value {
    assert_eq!(messages.len(), 1, "one answer expected: {messages:?}");
    messages.into_iter().next().expect("one answer")
}

/// The revision and active sequence a caller binds a request to.
fn context(s: &Session) -> Value {
    json!({"revision": s.revision.to_string(), "activeSequence": s.state.active_sequence.map(|id| id.0.to_string())})
}

/// The engine's result of an applied `result`.
fn applied(answer: &Value) -> Value {
    assert_eq!(answer["envelope"]["outcome"], json!("applied"), "{answer}");
    answer["envelope"]["result"].clone()
}

/// `execute` of `operation` bound to the session as it is, applied; answers the engine's result.
fn edit(worker: &mut Worker, request: &str, operation: &str, params: Value) -> Value {
    let expect = context(worker.session());
    let message = json!({"type": "execute", "requestId": request, "operation": operation, "params": params, "expect": expect, "deadlineMs": deadline()});
    applied(&one(worker.handle(&message)))
}

/// The result of job `request`, polled as the host polls.
fn job_result(worker: &mut Worker, request: &str) -> Value {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(1800) {
        if let Some(result) = worker.poll().into_iter().find(|m| m["type"] == "jobResult" && m["requestId"] == request) {
            return result;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("the job {request} has no result after 30 minutes");
}

/// The ffmpeg tool `name` (an external oracle, never linked): on PATH, else where Windows installs
/// usually put it.
fn tool(name: &str) -> PathBuf {
    let exe = format!("{name}.exe");
    let candidates = [PathBuf::from(name), Path::new("C:\\ffmpeg\\bin").join(&exe), Path::new("C:\\ProgramData\\chocolatey\\bin").join(&exe)];
    let works = |tool: &PathBuf| Command::new(tool).arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
    candidates.into_iter().find(works).unwrap_or_else(|| panic!("{name} is on PATH or in C:\\ffmpeg\\bin"))
}

/// What `command` writes on stdout; it must succeed.
fn run(command: &mut Command) -> Vec<u8> {
    let out = command.stderr(Stdio::inherit()).output().expect("the tool runs");
    assert!(out.status.success(), "the tool succeeds: {command:?}");
    out.stdout
}

/// ffprobe's JSON about `file`, with `args`.
fn probe(ffprobe: &Path, file: &Path, args: &[&str]) -> Value {
    let out = run(Command::new(ffprobe).args(["-v", "error", "-of", "json"]).args(args).arg(file));
    serde_json::from_slice(&out).expect("ffprobe writes JSON")
}

/// The sound of `file` from the PTS `from` (seconds) for `seconds`, mono at [`RATE`], as ffmpeg
/// decodes it, its first sample at that PTS (what precedes it, an AAC priming included, is cut, and
/// a gap at the start is filled).
fn mono(ffmpeg: &Path, file: &Path, from: f64, seconds: f64) -> Vec<f32> {
    let mut command = Command::new(ffmpeg);
    command.args(["-v", "error", "-ss"]).arg(format!("{from:.6}")).arg("-i").arg(file).arg("-t").arg(format!("{seconds:.6}"));
    command.args(["-vn", "-af", "aresample=async=1:first_pts=0", "-ac", "1", "-ar", "48000", "-f", "f32le", "-"]);
    run(&mut command).chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()
}

/// The `scale` filter that turns YUV into RGB24 as `matrix` and `range` say.
fn to_rgb(matrix: &str, range: &str) -> String {
    format!("scale=in_color_matrix={matrix}:in_range={range},format=rgb24")
}

/// The frame of `file` at the PTS `at` (seconds), as RGB24 under `filter`.
fn rgb_frame(ffmpeg: &Path, file: &Path, at: f64, filter: &str) -> Vec<u8> {
    let mut command = Command::new(ffmpeg);
    command.args(["-v", "error", "-ss"]).arg(format!("{at:.6}")).arg("-i").arg(file);
    command.args(["-frames:v", "1", "-vf", filter, "-f", "rawvideo", "-"]);
    run(&mut command)
}

/// The PSNR of the RGB24 `rgb` against the RGBA `reference`, in dB (infinite when they are equal).
fn psnr(reference: &[u8], rgb: &[u8]) -> f64 {
    assert_eq!(reference.len() / 4, rgb.len() / 3, "the pictures have the same size");
    let mut sum = 0u64;
    for (r, p) in reference.chunks_exact(4).zip(rgb.chunks_exact(3)) {
        for c in 0..3 {
            let d = i64::from(r[c]) - i64::from(p[c]);
            sum += (d * d) as u64;
        }
    }
    if sum == 0 {
        return f64::INFINITY;
    }
    let mse = sum as f64 / rgb.len() as f64;
    10.0 * (255.0 * 255.0 / mse).log10()
}

/// The lag of `exported` against `reference` (in samples, positive when the export's sound is late)
/// at which `window` samples of the export from `from` match the reference best, searched within
/// [`LAG_SPAN`] of `offset` (where the export's first sample sits in the reference), and the
/// normalised correlation there.
fn best_lag(exported: &[f32], reference: &[f32], offset: usize, from: usize, window: usize) -> (i64, f64) {
    let x = &exported[from..from + window];
    let ex: f64 = x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum();
    let mut best = (0i64, f64::MIN);
    for at in offset + from - LAG_SPAN..=offset + from + LAG_SPAN {
        let y = &reference[at..at + window];
        let (mut xy, mut ey) = (0f64, 0f64);
        for (a, b) in x.iter().zip(y) {
            xy += f64::from(*a) * f64::from(*b);
            ey += f64::from(*b) * f64::from(*b);
        }
        let r = xy / (ex * ey).sqrt().max(1e-12);
        if r > best.1 {
            best = (at as i64 - (offset + from) as i64, r);
        }
    }
    (-best.0, best.1)
}

/// The smoke workspace, `<CARGO_TARGET_TMPDIR>/nw1a-bb5-smoke`, with BB5 hard-linked as its staged
/// file (`files/lts-<sha256>.mp4`) and whatever an earlier run left there removed, the link aside.
fn workspace(source: &Path) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("nw1a-bb5-smoke");
    std::fs::create_dir_all(root.join("files")).expect("the smoke workspace exists");
    for entry in std::fs::read_dir(&root).expect("the smoke workspace lists").filter_map(Result::ok) {
        let path = entry.path();
        if entry.file_name().to_str() == Some("files") {
            continue;
        }
        let removed = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        removed.unwrap_or_else(|e| panic!("{} from an earlier run is removed: {e}", path.display()));
    }
    let link = root.join("files").join(format!("lts-{BB5_SHA256}.mp4"));
    if !link.exists() {
        std::fs::hard_link(source, &link).expect("BB5 is hard-linked into the workspace (never copied)");
    }
    assert_eq!(link.metadata().expect("the link is there").len(), BB5_BYTES, "the link is BB5");
    root
}

#[test]
#[ignore = "needs BB5 (the LTS harness's Big Brother chunk, OC-1) and ffmpeg: run with --ignored --nocapture"]
fn bb5_scripted_smoke() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../../../../editor-harness-20261014/workspace/sources/lts-{BB5_SHA256}.mp4"));
    let size = std::fs::metadata(&source).unwrap_or_else(|e| panic!("BB5 is at {}: {e}", source.display())).len();
    assert_eq!(size, BB5_BYTES, "BB5 has its measured size");
    let (ffmpeg, ffprobe) = (tool("ffmpeg"), tool("ffprobe"));
    let root = workspace(&source);
    let started = Instant::now();

    // stage, place, fill, sheet
    let mut worker = Worker::new(&root).expect("the worker starts");
    let hello = json!({"type": "hello", "protocol": filmcraft_lts_worker::PROTOCOL, "mode": "session", "engine": "filmcraft", "sessionId": "s-bb5"});
    let ready = one(worker.handle(&hello));
    assert_eq!(ready["type"], json!("ready"), "{ready}");
    let handle = format!("sha256:{BB5_SHA256}");
    let stage = json!({"type": "stage", "stageId": "st-bb5", "handle": handle, "path": format!("files/lts-{BB5_SHA256}.mp4"), "bytes": BB5_BYTES});
    let staged = one(worker.handle(&stage));
    assert_eq!(staged["ok"], json!(true), "BB5 is staged by its sha256: {staged}");
    edit(&mut worker, "bb-seq", "file.newSequence", json!({"name": "BB5 smoke", "width": 1080, "height": 1920, "fps": 25}));
    let expect = context(worker.session());
    let import = json!({"type": "import", "requestId": "bb-import", "handle": handle, "expect": expect, "deadlineMs": deadline()});
    let imported = one(worker.handle(&import));
    let item = applied(&imported)["items"][0].as_u64().unwrap_or_else(|| panic!("BB5 imports: {imported}"));
    let frame = TICKS_PER_SECOND / 25;
    let place = json!({"item": item, "track": "V1", "time": 0, "sourceIn": SOURCE_IN * frame, "duration": FRAMES * frame});
    let placed = edit(&mut worker, "bb-place", "timeline.place", place);
    let clip = placed["clips"][0].as_u64().unwrap_or_else(|| panic!("the placed clip: {placed}"));
    edit(&mut worker, "bb-fill", "clip.fillFrame", json!({"clips": [clip]}));
    let at: Vec<Value> = (0..9).map(|k| json!({"frame": k * (FRAMES - 1) / 8})).collect();
    let params = json!({"at": at, "maxSide": 256, "format": "jpeg"});
    let sheet = json!({"type": "read", "readId": "bb-sheet", "kind": "frames", "params": params, "deadlineMs": deadline()});
    let sheet = one(worker.handle(&sheet));
    assert_eq!(sheet["ok"], json!(true), "the sheet reads: {sheet}");

    // the map: sequence frame k shows source frame 1000 + k
    let s = worker.session();
    let sequence = s.active_sequence().expect("the smoke sequence is open");
    let (_, placed_clip) = sequence.find_item(ClipId(clip)).expect("the clip is on the sequence");
    let unmapped: Vec<i64> = (0..FRAMES).filter(|&k| placed_clip.source_time_at(Tick(k * frame)) != Tick((SOURCE_IN + k) * frame)).collect();
    let engine_video = s.project.item(ItemId(item)).and_then(|it| it.as_media()).map(|m| json!(m.info.video)).unwrap_or_default();
    let rate = s.sequence_rate();

    // export, probe
    let expect = context(worker.session());
    let params = json!({"format": "h264"});
    let job = json!({"type": "job", "requestId": "bb-export", "kind": "export_media", "params": params, "expect": expect, "deadlineMs": deadline()});
    let accepted = one(worker.handle(&job));
    assert_eq!(accepted["type"], json!("jobAccepted"), "{accepted}");
    let result = job_result(&mut worker, "bb-export");
    assert_eq!(result["ok"], json!(true), "the export finishes: {result}");
    let rel = result["outputs"][0]["path"].as_str().unwrap_or_else(|| panic!("an output: {result}")).to_string();
    let output = root.join(&rel);
    let entries = "stream=index,codec_type,codec_name,width,height,r_frame_rate,avg_frame_rate,start_time,duration,nb_read_frames,\
                   color_range,color_space,color_transfer,color_primaries,sample_rate,channels";
    let streams = probe(&ffprobe, &output, &["-count_frames", "-show_entries", entries]);
    let list = streams["streams"].as_array().cloned().unwrap_or_default();
    let video = list.iter().find(|s| s["codec_type"] == "video").cloned().unwrap_or_default();
    let audio = list.iter().find(|s| s["codec_type"] == "audio").cloned().unwrap_or_default();
    let times = run(Command::new(&ffprobe).args(["-v", "error", "-select_streams", "v:0", "-show_entries", "frame=pts_time", "-of", "csv=p=0"]).arg(&output));
    let times: Vec<f64> = String::from_utf8_lossy(&times).lines().filter_map(|l| l.trim().trim_end_matches(',').parse().ok()).collect();
    let gaps: Vec<usize> = (1..times.len()).filter(|&k| (times[k] - times[k - 1] - 0.04).abs() > 1e-3).collect();

    // each frame against the project render, under the export's own colour tags
    let tag = |key: &str, unknown: &str| video[key].as_str().filter(|v| *v != "unknown").unwrap_or(unknown).to_string();
    let (matrix, range) = (tag("color_space", "bt709"), tag("color_range", "tv"));
    let filter = to_rgb(&matrix, &range);
    let mut decoder = Command::new(&ffmpeg);
    decoder.args(["-v", "error", "-i"]).arg(&output).args(["-vf", filter.as_str(), "-f", "rawvideo", "-"]).stdout(Stdio::piped());
    let mut decoding = decoder.spawn().expect("ffmpeg decodes the export");
    let mut decoded = decoding.stdout.take().expect("ffmpeg's output is piped");
    let mut picture = vec![0u8; 1080 * 1920 * 3];
    let mut psnrs = Vec::new();
    for k in 0..FRAMES {
        if decoded.read_exact(&mut picture).is_err() {
            break;
        }
        let render = worker.session().try_render_program_at(1.0, rate.tick_of(k)).expect("the engine renders the frame");
        assert_eq!((render.w, render.h), (1080, 1920), "the project renders at the sequence's size");
        psnrs.push(psnr(&render.over_black_rgba8(), &picture));
    }
    drop(decoded);
    let _ = decoding.wait();
    let worst = psnrs.iter().copied().enumerate().min_by(|a, b| a.1.total_cmp(&b.1)).unwrap_or((0, f64::NAN));
    let finite: Vec<f64> = psnrs.iter().copied().filter(|p| p.is_finite()).collect();
    let mean = finite.iter().sum::<f64>() / finite.len().max(1) as f64;

    // audioStart: the export's sound against the source's at the same PTS (40 s + t), by cross-correlation
    let seconds = |key: &str, stream: &Value| stream[key].as_str().and_then(|v| v.parse::<f64>().ok()).unwrap_or(f64::NAN);
    let ticks = |s: f64| (s * TICKS_PER_SECOND as f64).round();
    let start = SOURCE_IN as f64 / 25.0;
    let exported = if audio.is_null() { Vec::new() } else { mono(&ffmpeg, &output, 0.0, FRAMES as f64 / 25.0) };
    let reference = mono(&ffmpeg, &source, start - 0.2, FRAMES as f64 / 25.0 + 0.4);
    let measured = (exported.len() >= 3 * RATE && reference.len() >= 4 * RATE).then(|| best_lag(&exported, &reference, RATE / 5, RATE, 2 * RATE));
    let (lag, correlation) = measured.unwrap_or((i64::MAX, f64::NAN));

    // the colour interpretation of the untagged source: the engine's render of source frame 1000 at
    // 1:1 against ffmpeg's decode of it as limited and as full range
    edit(&mut worker, "bb-src-seq", "file.newSequence", json!({"name": "BB5 source", "width": 1280, "height": 720, "fps": 25}));
    let place = json!({"item": item, "track": "V1", "time": 0, "sourceIn": SOURCE_IN * frame, "duration": frame});
    edit(&mut worker, "bb-src-place", "timeline.place", place);
    let engine = worker.session().try_render_program_at(1.0, Tick::ZERO).expect("the engine renders source frame 1000").over_black_rgba8();
    let as_limited = psnr(&engine, &rgb_frame(&ffmpeg, &source, start, &to_rgb("bt709", "tv")));
    let as_full = psnr(&engine, &rgb_frame(&ffmpeg, &source, start, &to_rgb("bt709", "pc")));
    let source_tags = probe(&ffprobe, &source, &["-select_streams", "v:0", "-show_entries", "stream=color_range,color_space,color_transfer,color_primaries"]);

    let report = json!({
        "test": "T-NW-SMOKE-BB5",
        "source": {"sha256": BB5_SHA256, "bytes": BB5_BYTES, "sourceIn": SOURCE_IN, "frames": FRAMES, "warnings": imported["media"][0]["warnings"]},
        "export": {"path": rel, "sha256": result["outputs"][0]["sha256"], "bytes": result["outputs"][0]["bytes"], "revision": result["revision"]},
        "picture": {"frames": times.len(), "rFrameRate": video["r_frame_rate"], "avgFrameRate": video["avg_frame_rate"], "ptsGapsAfter": gaps,
                    "duration": video["duration"], "width": video["width"], "height": video["height"]},
        "sound": {"codec": audio["codec_name"], "sampleRate": audio["sample_rate"], "channels": audio["channels"], "duration": audio["duration"]},
        "psnr": {"floor": PSNR_FLOOR, "frames": psnrs.len(), "min": worst.1, "minAt": worst.0, "mean": mean, "against": "project render"},
        "map": {"checked": FRAMES, "unmapped": unmapped},
        "avOffset": {"videoStartTick": ticks(seconds("start_time", &video)), "audioStartTick": ticks(seconds("start_time", &audio)),
                     "lagSamples": lag, "residualTicks": ticks(lag as f64 / RATE as f64), "correlation": correlation,
                     "method": "ffprobe_start+xcorr", "reference": "the source's sound at PTS 40 s + t"},
        "colour": {"export": {"matrix": matrix, "range": range, "transfer": video["color_transfer"], "primaries": video["color_primaries"]},
                   "source": {"tags": source_tags["streams"][0], "engine": engine_video, "psnrAsLimited": as_limited, "psnrAsFull": as_full,
                              "engineReads": if as_limited >= as_full { "limited" } else { "full" }}},
        "seconds": started.elapsed().as_secs_f64(),
    });
    println!("T-NW-SMOKE-BB5 report: {report:#}");
    let written = serde_json::to_vec_pretty(&report).expect("the report serializes");
    std::fs::write(root.join("bb5-smoke-report.json"), written).expect("the report is written");
    // the export is measured: its bytes go (the report keeps its sha256)
    std::fs::remove_dir_all(root.join("outputs")).expect("the export is removed");

    assert!(unmapped.is_empty(), "sequence frame k shows source frame {SOURCE_IN} + k, except at {unmapped:?}");
    assert_eq!(times.len(), FRAMES as usize, "ffprobe decodes 300 frames of the export");
    assert_eq!(video["r_frame_rate"], json!("25/1"), "the export runs at 25/1");
    assert!(gaps.is_empty(), "consecutive PTS at 25/1, except after frames {gaps:?}");
    assert_eq!(psnrs.len(), FRAMES as usize, "every export frame is compared");
    assert!(worst.1 >= PSNR_FLOOR, "every export frame matches the project render above {PSNR_FLOOR} dB, not frame {}: {} dB", worst.0, worst.1);
    assert!(correlation > 0.5, "the export's sound is the source's (correlation {correlation})");
    assert!(lag.unsigned_abs() <= LAG_SPAN as u64 / 2, "the export's sound is within half a frame of the source's PTS: {lag} samples");
}
