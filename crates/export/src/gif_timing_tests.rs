//! Read exported GIFs through a decoder and external oracles; do not test a copy of the scheduler.

use super::*;
use image::AnimationDecoder;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicUsize;

fn output(name: &str) -> PathBuf {
    filmcraft_testkit::temp_path(&filmcraft_testkit::fixtures_dir("export/gif-timing").join(name))
}

fn fixture(rate: FrameRate, first: i64, count: i64, path: &Path) -> (Arc<Project>, ItemId, filmcraft_render::SourceMap, ExportSettings) {
    let (mut project, seq, sources) = tests::project();
    let q = Arc::make_mut(&mut project).sequence_mut(seq).unwrap();
    q.video_tracks[0].items[0].duration = Tick(2 * filmcraft_time::TICKS_PER_SECOND);
    q.audio_tracks[0].items[0].duration = Tick(2 * filmcraft_time::TICKS_PER_SECOND);
    let start = rate.tick_of(first);
    let settings = ExportSettings {
        format: Format::Gif,
        path: path.to_string_lossy().into_owned(),
        frame_size: Some((64, 36)),
        frame_rate: Some(rate),
        range: Some(TimeRange::new(start, rate.tick_of(first + count) - start)),
        ..Default::default()
    };
    (project, seq, sources, settings)
}

fn decoded_delays(path: &Path) -> Vec<u64> {
    let decoder = image::codecs::gif::GifDecoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap())).unwrap();
    let frames = decoder.into_frames().collect_frames().unwrap();
    frames
        .iter()
        .map(|frame| {
            assert_eq!(frame.buffer().dimensions(), (64, 36));
            let pixel = frame.buffer().get_pixel(32, 18).0;
            assert!(pixel[0] > 230 && pixel[1] < 30 && pixel[2] < 30, "decoded red fixture: {pixel:?}");
            let (numerator, denominator) = frame.delay().numer_denom_ms();
            assert_eq!(u64::from(numerator) % (10 * u64::from(denominator)), 0, "GIF delays are whole centiseconds");
            u64::from(numerator) / (10 * u64::from(denominator))
        })
        .collect()
}

fn assert_prefix_timing(delays: &[u64], rate: FrameRate, count: usize) {
    assert_eq!(delays.len(), count);
    let mut total = 0u128;
    for (index, delay) in delays.iter().enumerate() {
        assert!((1..=u64::from(u16::MAX)).contains(delay));
        total += u128::from(*delay);
        let actual = total * u128::try_from(rate.num).unwrap();
        let ideal = (index as u128 + 1) * 100 * u128::try_from(rate.den).unwrap();
        // In rational centiseconds, each cumulative boundary must be at most half a tick away.
        let twice_error = 2 * actual.abs_diff(ideal);
        let denominator = u128::try_from(rate.num).unwrap();
        assert!(twice_error <= denominator, "frame {}: {total}cs for {rate:?}", index + 1);
        if twice_error == denominator {
            assert!(actual >= ideal, "half-centisecond tie at frame {} must round upward", index + 1);
        }
    }
}

#[test]
fn gif_timing_decoded_prefixes_preserve_frame_grid_duration_across_batches() {
    for (case, rate, first, count) in [
        ("24", FrameRate::FPS_24, 0, 48),
        ("24-large-rational", FrameRate { num: 2_400_000_000, den: 100_000_000 }, 0, 48),
        ("24-range", FrameRate::FPS_24, 7, 24),
        ("25", FrameRate::FPS_25, 0, 48),
        ("23976-range", FrameRate::FPS_23_976, 7, 36),
        ("2997-range", FrameRate::FPS_29_97, 11, 45),
        ("60", FrameRate::FPS_60, 0, 120),
        ("100", FrameRate::new(100, 1), 0, 20),
        ("max-delay", FrameRate::new(100, i64::from(u16::MAX)), 0, 1),
    ] {
        for threads in [2, 3] {
            let path = output(&format!("{case}-{threads}.gif"));
            let (project, seq, sources, settings) = fixture(rate, first, count, &path);
            let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
            let result = pool.install(|| export(&project, seq, &settings, &sources, &Progress::default())).unwrap();
            assert_eq!(result.frames, count as u64);
            let delays = decoded_delays(&path);
            assert_prefix_timing(&delays, rate, count as usize);
            if case == "24" {
                assert_eq!(delays.iter().sum::<u64>(), 200);
                assert_eq!(delays.iter().filter(|&&delay| delay == 4).count(), 40);
                assert_eq!(delays.iter().filter(|&&delay| delay == 5).count(), 8);
                assert_eq!(&delays[..6], &[4, 4, 5, 4, 4, 4], "half-centisecond ties round upward");
            }
        }
    }
}

fn assert_rejected_rate(case: &str, rate: FrameRate) {
    for existing in [false, true] {
        let path = output(&format!("{case}-{existing}.gif"));
        if existing {
            std::fs::write(&path, b"preserve this previous output").unwrap();
        }
        let (project, seq, sources, settings) = fixture(rate, 0, 1, &path);
        let requests = AtomicUsize::new(0);
        let source = |id| {
            requests.fetch_add(1, Ordering::Relaxed);
            sources.source(id)
        };
        let progress = Progress::default();
        let result = export(&project, seq, &settings, &source, &progress);
        assert!(matches!(result, Err(ExportError::Unsupported(_))), "{case}: {result:?}");
        assert_eq!(requests.load(Ordering::Relaxed), 0, "source queried before rejecting {case}");
        assert_eq!(progress.done.load(Ordering::Relaxed), 0);
        assert!(!progress.finished.load(Ordering::Relaxed));
        if existing {
            assert_eq!(std::fs::read(&path).unwrap(), b"preserve this previous output");
        } else {
            assert!(!path.exists(), "invalid timing created an output");
        }
    }
}

#[test]
fn gif_timing_zero_delay_rate_fails_before_render_or_output() {
    assert_rejected_rate("zero-delay", FrameRate::FPS_120);
}

#[test]
fn gif_timing_overflow_delay_rate_fails_before_render_or_output() {
    assert_rejected_rate("overflow-delay", FrameRate::new(1, 700));
}

#[test]
fn gif_timing_external_oracles_decode_exact_duration() {
    let (ffmpeg, ffprobe) = filmcraft_testkit::require_oracles!();
    let path = output("oracle-24.gif");
    let (project, seq, sources, settings) = fixture(FrameRate::FPS_24, 0, 48, &path);
    export(&project, seq, &settings, &sources, &Progress::default()).unwrap();
    let probe = std::process::Command::new(ffprobe)
        .args(["-v", "error", "-min_delay", "0", "-ignore_loop", "1", "-i"])
        .arg(&path)
        .args([
            "-select_streams",
            "v:0",
            "-show_entries",
            "frame=pts_time,duration_time,pkt_duration_time:stream=width,height,nb_frames,duration",
            "-of",
            "json",
        ])
        .output()
        .unwrap();
    assert!(probe.status.success(), "{}", String::from_utf8_lossy(&probe.stderr));
    let json: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
    let frames = json["frames"].as_array().unwrap();
    let delays: Vec<u64> = frames
        .iter()
        .map(|frame| {
            let duration = frame["duration_time"].as_str().or_else(|| frame["pkt_duration_time"].as_str()).expect("ffprobe frame duration is required");
            (duration.parse::<f64>().unwrap() * 100.0).round() as u64
        })
        .collect();
    assert_prefix_timing(&delays, FrameRate::FPS_24, 48);
    assert_eq!(delays.iter().sum::<u64>(), 200);
    let stream = &json["streams"][0];
    assert_eq!(stream["width"], 64);
    assert_eq!(stream["height"], 36);
    assert_eq!(stream["nb_frames"], "48");
    assert_eq!(stream["duration"], "2.000000");
    let decode = std::process::Command::new(ffmpeg)
        .args(["-v", "error", "-xerror", "-min_delay", "0", "-ignore_loop", "1", "-i"])
        .arg(&path)
        .args(["-f", "null", "-"])
        .output()
        .unwrap();
    assert!(decode.status.success(), "{}", String::from_utf8_lossy(&decode.stderr));
}
