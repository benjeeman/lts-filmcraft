//! Newsroom material the MP4 and MOV reader gets wrong (batch BF-2 of the LTS editor bug fixes,
//! OC-18; master plan v3 W7; `logs/lts-media/real-material-gaps.md`, the Film table, read from the
//! code). Each test builds its file here with the crate's own muxer and encoders, as its unit tests
//! do: no media file is read from disk and nothing is downloaded.
//!
//! | Test | Gap | What it pins |
//! |---|---|---|
//! | `duration_is_the_longest_stream` | video shorter than audio (news13) | the file lasts as long as its longest stream |
//! | `every_audio_track_is_read` | several or mono audio tracks | the channels of every audio track, in track order |
//! | `aac_51_plays_in_the_engine_channel_order` | AAC 5.1 | AAC element order reordered to L, R, C, LFE, Ls, Rs |
//! | `interlaced_and_high_422_h264_are_refused_at_open` | interlaced and 10-bit 4:2:2 H.264 | a refusal naming why, at open, not black frames |
//! | `a_cut_faststart_file_is_clamped_to_what_is_there` | cut fast-start download | the duration ends where the readable samples end |
//! | `a_damaged_sample_is_concealed_and_decoding_moves_on` | damaged streams | a sample that fails shows the last good picture |
//! | `every_audio_track_keeps_its_own_layout` | 5.1 and stereo tracks, six mono tracks | the channel count of each track, in order |
//! | `matroska_reads_every_audio_track` | several audio tracks (Matroska) | every track's channels, each its own track |
//! | `interlaced_h264_in_matroska_is_refused_at_open` | interlaced H.264 (Matroska) | a refusal naming why, at open |
//! | `avc_intra_and_interlaced_h264_in_mxf_are_refused_and_the_sound_plays` | AVC-Intra, interlaced (MXF) | picture refused, saying why |
//! | `a_transport_stream_reads_every_audio_stream_and_lasts_as_long_as_its_sound` | TS: one sound, picture's length | every stream; the sound's end |
//! | `interlaced_h264_in_a_transport_stream_is_refused_and_the_sound_plays` | interlaced H.264 (TS) | picture refused, saying why |
//!
//! The engine's half of the batch (the damage count, the 5.1 downmix, a capture imported in the
//! background, the warning on a cut file) is `crates/engine/tests/real_material_import.rs`.

use std::io::Cursor;
use std::sync::Arc;

use filmcraft_bitstream::{BitWriter, escape_rbsp};
use filmcraft_codecs::{CodecError, MkvSource, Mp4Source, MpegSource, MxfSource};
use filmcraft_isobmff::{AvcConfig, Brand, FourCc, Mp4Writer, PcmConfig, SampleEntry, TrackConfig, WriteSample, WriterOptions};
use filmcraft_matroska::{MkvWriter, MuxOptions, TrackKind, TrackSpec};
use filmcraft_media::{FrameRequest, MediaError, MediaSource};
use filmcraft_mxf::{FrameInfo, MxfWriter, PackageIds, Pattern, PictureCoding, PictureDesc, Rational, SoundDesc, WriterConfig};
use filmcraft_time::{TICKS_PER_SECOND, Tick};

/// The audio rate of every file here.
const RATE: u32 = 48_000;

// ------------------------------------------------------------------------------------ fixtures

/// A 64×32 ProRes 422 HQ frame whose luma (10 bits) is `luma` everywhere.
fn prores_frame(luma: u16) -> Vec<u8> {
    let mut frame = filmcraft_prores::Frame::new(64, 32, filmcraft_prores::ChromaFormat::Yuv422, 10, false);
    frame.y.fill(luma);
    filmcraft_prores::Encoder::new(filmcraft_prores::Profile::Hq, 64, 32).encode(&frame).expect("the frame encodes")
}

/// A ProRes 422 HQ video track of 64×32 at 25 fps.
fn prores_track() -> TrackConfig {
    TrackConfig::new(SampleEntry::prores(FourCc(*b"apch"), 64, 32), 25)
}

/// One sample of `data` lasting `duration` units of its track.
fn sample(data: &[u8], duration: u32) -> WriteSample<'_> {
    WriteSample { data, duration, composition_offset: 0, is_sync: true }
}

/// Mono 16-bit little-endian PCM at [`RATE`].
fn mono16() -> PcmConfig {
    PcmConfig { bits: 16, float: false, big_endian: false, signed: true, channels: 1, sample_rate: f64::from(RATE) }
}

/// A tone of `freq` Hz at `level`, `samples` long, at [`RATE`].
fn tone(freq: f64, level: f32, samples: usize) -> Vec<f32> {
    (0..samples).map(|i| level * (2.0 * std::f64::consts::PI * freq * i as f64 / f64::from(RATE)).sin() as f32).collect()
}

/// `samples` as mono 16-bit little-endian PCM bytes.
fn pcm16(samples: &[f32]) -> Vec<u8> {
    samples.iter().flat_map(|s| ((s.clamp(-1.0, 1.0) * 32_767.0) as i16).to_le_bytes()).collect()
}

/// AAC access units of the planar `channels` at [`RATE`], `bitrate` bits a second; answers them and
/// the AudioSpecificConfig.
fn aac(channels: &[Vec<f32>], bitrate: u32) -> (Vec<Vec<u8>>, Vec<u8>) {
    let config = filmcraft_aac::EncoderConfig::cbr(RATE, channels.len(), bitrate);
    let mut encoder = filmcraft_aac::Encoder::new(config).expect("an AAC encoder");
    let planes: Vec<&[f32]> = channels.iter().map(Vec::as_slice).collect();
    let mut units = encoder.encode(&planes);
    units.extend(encoder.flush());
    (units, encoder.audio_specific_config())
}

/// The energy of `x` at `freq` Hz (Goertzel), at [`RATE`].
fn tone_power(x: &[f32], freq: f64) -> f64 {
    let coeff = 2.0 * (2.0 * std::f64::consts::PI * freq / f64::from(RATE)).cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &v in x {
        let s0 = f64::from(v) + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    s1 * s1 + s2 * s2 - coeff * s1 * s2
}

/// Which of `freqs` carries the most energy in `x`, and how many times more than the next one does.
fn dominant(x: &[f32], freqs: &[f64]) -> (f64, f64) {
    let mut powers: Vec<(f64, f64)> = freqs.iter().map(|&f| (f, tone_power(x, f))).collect();
    powers.sort_by(|a, b| b.1.total_cmp(&a.1));
    (powers[0].0, powers[0].1 / powers[1].1.max(1e-12))
}

/// Checks that channel `c` of `window` carries the tone `freq` of `freqs` and no other.
fn assert_tone(window: &[Vec<f32>], c: usize, freq: f64, freqs: &[f64]) {
    let (found, ratio) = dominant(&window[c], freqs);
    assert!((found - freq).abs() < 0.5 && ratio > 10.0, "channel {c} carries the {freq} Hz tone, not {found} Hz (ratio {ratio})");
}

/// An SPS NAL unit (ITU-T H.264 §7.3.2.1.1) of a 16-pixel-wide picture: `profile`, chroma format
/// `chroma`, `bits` per sample, and progressive frames (`interlaced: None`) or field coding, with
/// MBAFF or with field pictures (`Some(mbaff)`).
fn sps(profile: u8, chroma: u32, bits: u32, interlaced: Option<bool>) -> Vec<u8> {
    let mut w = filmcraft_bitstream::BitWriter::new();
    w.write_bits(0x67, 8);
    w.write_bits(u32::from(profile), 8);
    w.write_bits(0, 8); // constraint flags
    w.write_bits(30, 8); // level 3.0
    w.write_ue(0); // seq_parameter_set_id
    w.write_ue(chroma); // chroma_format_idc (every profile used here is a High one)
    if chroma == 3 {
        w.write_bit(false); // separate_colour_plane_flag
    }
    w.write_ue(bits - 8); // bit_depth_luma_minus8
    w.write_ue(bits - 8); // bit_depth_chroma_minus8
    w.write_bits(0, 2); // no transform bypass, no scaling matrices
    w.write_ue(0); // log2_max_frame_num_minus4
    w.write_ue(2); // pic_order_cnt_type
    w.write_ue(1); // max_num_ref_frames
    w.write_bit(false); // gaps_in_frame_num_value_allowed_flag
    w.write_ue(0); // pic_width_in_mbs_minus1
    w.write_ue(0); // pic_height_in_map_units_minus1
    match interlaced {
        None => w.write_bit(true),
        Some(mbaff) => {
            w.write_bit(false); // frame_mbs_only_flag: field coded
            w.write_bit(mbaff); // mb_adaptive_frame_field_flag
        }
    }
    w.write_bit(true); // direct_8x8_inference_flag
    w.write_bit(false); // frame_cropping_flag
    w.write_bit(false); // vui_parameters_present_flag
    w.rbsp_trailing();
    w.finish()
}

/// A one-frame MP4 whose H.264 track carries `sps`; its sample is never decoded.
fn h264_mp4(sps: Vec<u8>) -> Arc<[u8]> {
    let pps = vec![0x68, 0xCE, 0x38, 0x80];
    let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).expect("a writer");
    let track = mux.add_track(TrackConfig::new(SampleEntry::avc(AvcConfig::new(vec![sps], vec![pps], 4), 16, 32), 25)).expect("a track");
    mux.write_sample(track, sample(&[0, 0, 0, 2, 0x65, 0x88], 1)).expect("a sample");
    mux.finish().expect("the file").into_inner().into()
}

// ------------------------------------------------------------------------------------ tests

/// The video stops at one second and the sound runs for three (news13: 24 s of video in a 12-minute
/// container). The file lasts as long as its longest stream, so the sound after the picture's end
/// can be placed and heard; today the MP4 duration is the video track's.
#[test]
fn duration_is_the_longest_stream() {
    let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mov)).expect("a writer");
    let video = mux.add_track(prores_track()).expect("a video track");
    let audio = mux.add_track(TrackConfig::new(SampleEntry::pcm(mono16()), RATE)).expect("an audio track");
    for k in 0..25u16 {
        mux.write_sample(video, sample(&prores_frame(100 + 20 * k), 1)).expect("a frame");
    }
    let sound = pcm16(&tone(440.0, 0.3, RATE as usize * 3));
    mux.write_sample(audio, sample(&sound, 1)).expect("three seconds of sound");
    let src = Mp4Source::open("short-picture.mov", mux.finish().expect("the file").into_inner().into()).expect("the file opens");
    assert_eq!(src.info().duration, Tick(3 * TICKS_PER_SECOND), "the file lasts as long as its longest stream");
    let late = src.audio(i64::from(RATE) * 5 / 2, 4_800, RATE).expect("the sound after the picture's end reads");
    let peak = late.channels[0].iter().fold(0f32, |m, v| m.max(v.abs()));
    assert!(peak > 0.2, "the sound is there after the picture ends: peak {peak}");
}

/// A MOV with four mono PCM tracks (a camera's four inputs) and an MP4 with two stereo AAC tracks
/// (a programme in two languages): the sound has the channels of every audio track, in track order,
/// so channel k carries what track k recorded; today only the first audio track is read.
#[test]
fn every_audio_track_is_read() {
    let mono = [300.0, 500.0, 700.0, 900.0];
    let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mov)).expect("a writer");
    let video = mux.add_track(prores_track()).expect("a video track");
    let mut tracks = Vec::new();
    for _ in mono {
        tracks.push(mux.add_track(TrackConfig::new(SampleEntry::pcm(mono16()), RATE)).expect("a mono track"));
    }
    for k in 0..25u16 {
        mux.write_sample(video, sample(&prores_frame(100 + 20 * k), 1)).expect("a frame");
    }
    for (&track, &freq) in tracks.iter().zip(&mono) {
        mux.write_sample(track, sample(&pcm16(&tone(freq, 0.3, RATE as usize)), 1)).expect("a second of sound");
    }
    let src = Mp4Source::open("four-mono.mov", mux.finish().expect("the file").into_inner().into()).expect("the file opens");
    assert_eq!(src.info().audio.as_ref().map(|a| a.channels), Some(4), "every mono track is a channel of the sound");
    let window = src.audio(12_000, 4_800, RATE).expect("the sound reads").channels;
    for (c, &freq) in mono.iter().enumerate() {
        assert_tone(&window, c, freq, &mono);
    }

    let stereo = [400.0, 600.0, 800.0, 1000.0];
    let (hebrew_units, hebrew_config) = aac(&[tone(stereo[0], 0.3, RATE as usize), tone(stereo[1], 0.3, RATE as usize)], 128_000);
    let (english_units, english_config) = aac(&[tone(stereo[2], 0.3, RATE as usize), tone(stereo[3], 0.3, RATE as usize)], 128_000);
    let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).expect("a writer");
    let hebrew = mux.add_track(TrackConfig::new(SampleEntry::aac(hebrew_config, 2, RATE), RATE)).expect("the first language");
    let english = mux.add_track(TrackConfig::new(SampleEntry::aac(english_config, 2, RATE), RATE)).expect("the second language");
    for (track, units) in [(hebrew, &hebrew_units), (english, &english_units)] {
        for unit in units {
            mux.write_sample(track, sample(unit, 1024)).expect("an access unit");
        }
    }
    let src = Mp4Source::open("two-languages.mp4", mux.finish().expect("the file").into_inner().into()).expect("the file opens");
    assert_eq!(src.info().audio.as_ref().map(|a| a.channels), Some(4), "both stereo tracks are read");
    let window = src.audio(24_000, 4_800, RATE).expect("the sound reads").channels;
    for (c, &freq) in stereo.iter().enumerate() {
        assert_tone(&window, c, freq, &stereo);
    }
}

/// AAC 5.1 (channel configuration 6) codes its channels in element order: C, L, R, Ls, Rs, LFE. The
/// engine's 5.1 order is L, R, C, LFE, Ls, Rs (`filmcraft_audio_dsp::channels::Layout::Surround51`),
/// and a stereo track plays the first two: today that is C and L. The source reorders.
#[test]
fn aac_51_plays_in_the_engine_channel_order() {
    // C, L, R, Ls, Rs, LFE
    let coded = [1000.0, 300.0, 500.0, 700.0, 900.0, 80.0];
    let channels: Vec<Vec<f32>> = coded.iter().map(|&f| tone(f, 0.3, RATE as usize)).collect();
    let (units, config) = aac(&channels, 384_000);
    let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).expect("a writer");
    let track = mux.add_track(TrackConfig::new(SampleEntry::aac(config, 6, RATE), RATE)).expect("a 5.1 track");
    for unit in &units {
        mux.write_sample(track, sample(unit, 1024)).expect("an access unit");
    }
    let src = Mp4Source::open("surround.mp4", mux.finish().expect("the file").into_inner().into()).expect("the file opens");
    assert_eq!(src.info().audio.as_ref().map(|a| a.channels), Some(6), "six channels");
    let window = src.audio(24_000, 4_800, RATE).expect("the sound reads").channels;
    // L, R, C, LFE, Ls, Rs
    for (c, &freq) in [300.0, 500.0, 1000.0, 80.0, 700.0, 900.0].iter().enumerate() {
        assert_tone(&window, c, freq, &coded);
    }
}

/// The decoder implements progressive 8-bit 4:2:0 H.264 only. Interlaced coding (MBAFF or field
/// pictures, 1080i broadcast) and 10-bit or 4:2:2 coding (AVC-Intra, XAVC) used to open and then
/// decode to black frames; they are refused at open, saying why, so the import shows the reason.
#[test]
fn interlaced_and_high_422_h264_are_refused_at_open() {
    let control = Mp4Source::open("progressive.mp4", h264_mp4(sps(100, 1, 8, None))).expect("progressive 8-bit 4:2:0 H.264 opens");
    assert_eq!(control.info().video.as_ref().map(|v| v.pixel_format.as_str()), Some("YUV 4:2:0 8-bit"), "the control reads its SPS");
    let refused = [
        ("mbaff.mp4", sps(100, 1, 8, Some(true)), "interlaced"),
        ("fields.mp4", sps(100, 1, 8, Some(false)), "interlaced"),
        ("high-422-10.mp4", sps(122, 2, 10, None), "4:2:2"),
        ("high-422-8.mp4", sps(122, 2, 8, None), "4:2:2"),
        ("high-10.mp4", sps(110, 1, 10, None), "10-bit"),
    ];
    for (name, sps, why) in refused {
        let error = Mp4Source::open(name, h264_mp4(sps)).err().unwrap_or_else(|| panic!("{name} is refused at open"));
        match error {
            CodecError::Unsupported(message) => assert!(message.contains(why), "{name}: the refusal says {why}: {message}"),
            other => panic!("{name} is refused as unsupported, not {other:?}"),
        }
    }
}

/// A fast-start download cut at 60 % (the index in front is complete, the media data is not). It
/// opens, and lasts as long as its readable samples, so nothing past the cut is offered as a black
/// frame; today it opens at the full two seconds.
#[test]
fn a_cut_faststart_file_is_clamped_to_what_is_there() {
    let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mov)).expect("a writer");
    let video = mux.add_track(prores_track()).expect("a video track");
    let audio = mux.add_track(TrackConfig::new(SampleEntry::pcm(mono16()), RATE)).expect("an audio track");
    let sound = pcm16(&tone(440.0, 0.3, RATE as usize * 2));
    let per_frame = sound.len() / 50;
    for k in 0..50usize {
        mux.write_sample(video, sample(&prores_frame(100 + 15 * k as u16), 1)).expect("a frame");
        mux.write_sample(audio, sample(&sound[k * per_frame..(k + 1) * per_frame], 1)).expect("its sound");
    }
    let whole = mux.finish_faststart().expect("the file").into_inner();
    let full = Mp4Source::open("whole.mov", whole.clone().into()).expect("the whole file opens");
    assert_eq!(full.info().duration, Tick(2 * TICKS_PER_SECOND), "the whole file lasts two seconds");
    let cut: Arc<[u8]> = whole[..whole.len() * 6 / 10].into();
    let src = Mp4Source::open("cut.mov", cut).expect("a cut fast-start file still opens");
    let duration = src.info().duration;
    let bound = Tick(TICKS_PER_SECOND * 13 / 10);
    assert!(duration > Tick::ZERO && duration <= bound, "the duration ends where the readable samples end, not at {duration:?}");
    let last = Tick(duration.0 - TICKS_PER_SECOND / 25);
    assert!(src.video_frame(FrameRequest::full(last)).is_ok(), "the last frame within the cut decodes");
}

/// A sample that fails to decode (damaged in transfer) used to fail its frame request, which the
/// renderer turned into a black frame with nothing counted. The frame shows the last good picture
/// instead, and the frames after it decode as their own pictures.
#[test]
fn a_damaged_sample_is_concealed_and_decoding_moves_on() {
    let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mov)).expect("a writer");
    let video = mux.add_track(prores_track()).expect("a video track");
    for k in 0..10u16 {
        let data = if k == 4 { b"this is not a ProRes frame at all".to_vec() } else { prores_frame(100 + 70 * k) };
        mux.write_sample(video, sample(&data, 1)).expect("a sample");
    }
    let src = Mp4Source::open("damaged.mov", mux.finish().expect("the file").into_inner().into()).expect("a file with a bad sample opens");
    let luma = |k: i64| src.video_frame(FrameRequest::full(Tick(k * TICKS_PER_SECOND / 25))).map(|f| f.luma8());
    let before = luma(3).expect("frame 3 decodes");
    let damaged = luma(4).expect("the damaged frame is concealed, not an error");
    assert!(damaged == before, "the damaged frame shows the last good picture");
    for k in 5..10 {
        let frame = luma(k).unwrap_or_else(|e| panic!("frame {k} decodes after the damage: {e}"));
        assert!(frame != before, "frame {k} is its own picture");
    }
}

// ------------------------------------------------------------------------------------ more fixtures

/// An Annex B NAL unit of type `kind` (`nal_ref_idc` 3) holding `rbsp`, escaped.
fn annexb(kind: u8, rbsp: &[u8]) -> Vec<u8> {
    let mut nal = vec![0, 0, 0, 1, 0x60 | kind];
    nal.extend(escape_rbsp(rbsp));
    nal
}

/// A Baseline SPS NAL unit (its header byte first, as [`sps`] makes one) of a 16×16 progressive
/// picture whose output order is its decode order.
fn baseline_sps() -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_bits(0x67, 8);
    w.write_bits(66, 8); // profile_idc: Baseline
    w.write_bits(0, 8); // constraint flags
    w.write_bits(30, 8); // level 3.0
    w.write_ue(0); // seq_parameter_set_id
    w.write_ue(0); // log2_max_frame_num_minus4
    w.write_ue(2); // pic_order_cnt_type
    w.write_ue(1); // max_num_ref_frames
    w.write_bit(false); // gaps_in_frame_num_value_allowed_flag
    w.write_ue(0); // pic_width_in_mbs_minus1
    w.write_ue(0); // pic_height_in_map_units_minus1
    w.write_bit(true); // frame_mbs_only_flag
    w.write_bit(true); // direct_8x8_inference_flag
    w.write_bit(false); // frame_cropping_flag
    w.write_bit(false); // vui_parameters_present_flag
    w.rbsp_trailing();
    w.finish()
}

/// A PPS RBSP for CAVLC, with the deblocking controls in the slice header.
fn cavlc_pps() -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_ue(0); // pic_parameter_set_id
    w.write_ue(0); // seq_parameter_set_id
    w.write_bit(false); // entropy_coding_mode_flag: CAVLC
    w.write_bit(false); // bottom_field_pic_order_in_frame_present_flag
    w.write_ue(0); // num_slice_groups_minus1
    w.write_ue(0); // num_ref_idx_l0_default_active_minus1
    w.write_ue(0); // num_ref_idx_l1_default_active_minus1
    w.write_bit(false); // weighted_pred_flag
    w.write_bits(0, 2); // weighted_bipred_idc
    w.write_se(0); // pic_init_qp_minus26
    w.write_se(0); // pic_init_qs_minus26
    w.write_se(0); // chroma_qp_index_offset
    w.write_bit(true); // deblocking_filter_control_present_flag
    w.write_bit(false); // constrained_intra_pred_flag
    w.write_bit(false); // redundant_pic_cnt_present_flag
    w.rbsp_trailing();
    w.finish()
}

/// An IDR access unit of the SPS NAL unit `sps` (its header byte first) and [`cavlc_pps`], whose
/// slice codes the 16×16 picture as one I_PCM macroblock with every sample `level`.
fn idr_access_unit(sps: &[u8], idr_pic_id: u32, level: u8) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_ue(0); // first_mb_in_slice
    w.write_ue(7); // slice_type: I, as every slice of the picture
    w.write_ue(0); // pic_parameter_set_id
    w.write_bits(0, 4); // frame_num
    w.write_ue(idr_pic_id);
    w.write_bit(false); // no_output_of_prior_pics_flag
    w.write_bit(false); // long_term_reference_flag
    w.write_se(0); // slice_qp_delta
    w.write_ue(0); // disable_deblocking_filter_idc
    w.write_se(0); // slice_alpha_c0_offset_div2
    w.write_se(0); // slice_beta_offset_div2
    w.write_ue(25); // mb_type: I_PCM
    w.align_zero(); // pcm_alignment_zero_bit
    w.write_bytes(&[level; 384]);
    w.rbsp_trailing();
    let mut au = annexb(7, sps.get(1..).unwrap_or_default());
    au.extend(annexb(8, &cavlc_pps()));
    au.extend(annexb(5, &w.finish()));
    au
}

/// A Matroska file of `tracks` with the frames `(track, millisecond, data)`, written in order.
fn matroska(tracks: Vec<TrackSpec>, frames: &[(usize, i64, Vec<u8>)]) -> Arc<[u8]> {
    let mut mux = MkvWriter::new(Cursor::new(Vec::new()), tracks, MuxOptions::default()).expect("a writer");
    for (track, ms, data) in frames {
        mux.write_frame(*track, ms * 1_000_000, true, data, None).expect("a frame");
    }
    mux.finish().expect("the file").into_inner().into()
}

/// A Matroska audio track of 16-bit little-endian PCM, `channels` at [`RATE`].
fn matroska_pcm(channels: u64) -> TrackSpec {
    let mut spec = TrackSpec::new(TrackKind::Audio, "A_PCM/INT/LIT");
    spec.audio = Some((f64::from(RATE), channels, Some(16)));
    spec
}

/// An MXF OP1a file of 25 pictures at 25 fps, each the H.264 access unit `picture` that `desc`
/// describes, with a second of a 440 Hz tone on a mono 16-bit sound track.
fn avc_mxf(picture: &[u8], desc: PictureDesc) -> Arc<[u8]> {
    let mut cfg = WriterConfig::new(Pattern::Op1a, Rational::new(25, 1), PackageIds::from_seed("bf2-mxf", "BF-2"));
    cfg.picture = Some(desc);
    cfg.sound = vec![SoundDesc { sample_rate: RATE, channels: 1, bits: 16 }];
    let mut mux = MxfWriter::new(Cursor::new(Vec::new()), cfg).expect("a writer");
    let sound = pcm16(&tone(440.0, 0.3, RATE as usize));
    for unit in sound.chunks(sound.len() / 25) {
        mux.push_picture(picture, FrameInfo::intra()).expect("a picture");
        mux.push_sound(0, unit).expect("its sound");
    }
    mux.finish().expect("the file").into_inner().into()
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

/// The PID of a capture's programme map, of its picture and of its two sounds.
const PMT_PID: u16 = 0x0100;
const VIDEO_PID: u16 = 0x1011;
const AUDIO_PIDS: [u16; 2] = [0x1100, 0x1101];

/// The programme association section (without its CRC): programme 1, its map at [`PMT_PID`].
const PAT: [u8; 12] = [0x00, 0xB0, 13, 0x00, 0x01, 0xC1, 0x00, 0x00, 0x00, 0x01, 0xE0 | (PMT_PID >> 8) as u8, PMT_PID as u8];

/// The programme map section (without its CRC) of a Blu-ray style capture: an `HDMV`
/// registration, so stream type 0x80 is LPCM; H.264 at [`VIDEO_PID`] and LPCM at each of
/// [`AUDIO_PIDS`].
fn capture_pmt() -> Vec<u8> {
    let mut body = vec![0x00, 0x01, 0xC1, 0x00, 0x00, 0xE0 | (VIDEO_PID >> 8) as u8, VIDEO_PID as u8, 0xF0, 6, 0x05, 4, b'H', b'D', b'M', b'V'];
    for (kind, pid) in [(0x1B, VIDEO_PID), (0x80, AUDIO_PIDS[0]), (0x80, AUDIO_PIDS[1])] {
        body.extend_from_slice(&[kind, 0xE0 | (pid >> 8) as u8, pid as u8, 0xF0, 0x00]);
    }
    let mut section = vec![0x02, 0xB0, (body.len() + 4) as u8];
    section.extend(body);
    section
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

/// A Blu-ray style transport stream capture: `frames` pictures at 25 fps from PTS 1 s, each the
/// H.264 access unit `picture`, and `seconds` of 16-bit stereo LPCM at 48 kHz from the same PTS on
/// each of two sound streams, one PES packet per 10 ms; the sounds are tones of `freqs` (the left
/// and right of the first stream, then of the second).
fn capture(picture: &[u8], frames: u64, seconds: usize, freqs: [f64; 4]) -> Arc<[u8]> {
    let mut out = psi_packet(0x0000, &PAT);
    out.extend(psi_packet(PMT_PID, &capture_pmt()));
    let mut cc = [0u8; 3];
    for k in 0..frames {
        packetize(VIDEO_PID, &pes(0xE0, 90_000 + 3_600 * k, picture), &mut cc[0], &mut out);
    }
    for (s, &pid) in AUDIO_PIDS.iter().enumerate() {
        for unit in 0..seconds * 100 {
            // 1920 bytes of samples; stereo (3) at 48 kHz (1); 16 bits (1 << 6)
            let mut payload = vec![0x07, 0x80, 0x31, 0x40];
            for i in 0..480 {
                let t = (unit * 480 + i) as f64 / 48_000.0;
                for freq in [freqs[2 * s], freqs[2 * s + 1]] {
                    let v = (0.25 * (2.0 * std::f64::consts::PI * freq * t).sin() * 32_767.0) as i16;
                    payload.extend_from_slice(&v.to_be_bytes());
                }
            }
            packetize(pid, &pes(0xBD, 90_000 + 900 * unit as u64, &payload), &mut cc[1 + s], &mut out);
        }
    }
    out.into()
}

// ------------------------------------------------------------------------------------ more tests

/// A file whose sound holds several tracks keeps each track's channel count, in order, so a clip
/// plays its first track as that track is laid out: a 5.1 AAC track and a stereo one are tracks of
/// six and two channels, and the six mono inputs of a recorder are six tracks of one channel,
/// never one 5.1 track.
#[test]
fn every_audio_track_keeps_its_own_layout() {
    let surround: Vec<Vec<f32>> = [1000.0, 300.0, 500.0, 700.0, 900.0, 80.0].iter().map(|&f| tone(f, 0.3, RATE as usize)).collect();
    let (surround_units, surround_config) = aac(&surround, 384_000);
    let (stereo_units, stereo_config) = aac(&[tone(400.0, 0.3, RATE as usize), tone(600.0, 0.3, RATE as usize)], 128_000);
    let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).expect("a writer");
    let five_one = mux.add_track(TrackConfig::new(SampleEntry::aac(surround_config, 6, RATE), RATE)).expect("a 5.1 track");
    let stereo = mux.add_track(TrackConfig::new(SampleEntry::aac(stereo_config, 2, RATE), RATE)).expect("a stereo track");
    for (track, units) in [(five_one, &surround_units), (stereo, &stereo_units)] {
        for unit in units {
            mux.write_sample(track, sample(unit, 1024)).expect("an access unit");
        }
    }
    let src = Mp4Source::open("surround-and-stereo.mp4", mux.finish().expect("the file").into_inner().into()).expect("the file opens");
    assert_eq!(src.audio_tracks(), vec![6, 2], "a 5.1 track and a stereo track");
    assert_eq!(src.info().audio.as_ref().map(|a| a.channels), Some(8), "the sound holds the channels of both");

    let mut mux = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mov)).expect("a writer");
    let mut tracks = Vec::new();
    for _ in 0..6 {
        tracks.push(mux.add_track(TrackConfig::new(SampleEntry::pcm(mono16()), RATE)).expect("a mono track"));
    }
    for (k, &track) in tracks.iter().enumerate() {
        mux.write_sample(track, sample(&pcm16(&tone(300.0 + 100.0 * k as f64, 0.3, RATE as usize)), 1)).expect("a second of sound");
    }
    let src = Mp4Source::open("six-mono.mov", mux.finish().expect("the file").into_inner().into()).expect("the file opens");
    assert_eq!(src.audio_tracks(), vec![1; 6], "six mono inputs are six tracks of one channel");
}

/// A Matroska file with a mono and a stereo PCM track: the sound holds the channels of both, in
/// track order, each its own track (Matroska used to read the first audio track only).
#[test]
fn matroska_reads_every_audio_track() {
    let freqs = [300.0, 500.0, 700.0];
    let mono = tone(freqs[0], 0.3, RATE as usize);
    let (left, right) = (tone(freqs[1], 0.3, RATE as usize), tone(freqs[2], 0.3, RATE as usize));
    let mut frames = Vec::new();
    for block in 0..100usize {
        let span = block * 480..(block + 1) * 480;
        frames.push((0, 10 * block as i64, pcm16(&mono[span.clone()])));
        let stereo: Vec<f32> = span.flat_map(|i| [left[i], right[i]]).collect();
        frames.push((1, 10 * block as i64, pcm16(&stereo)));
    }
    let src = MkvSource::open("two-tracks.mkv", matroska(vec![matroska_pcm(1), matroska_pcm(2)], &frames)).expect("the file opens");
    assert_eq!(src.audio_tracks(), vec![1, 2], "a mono track and a stereo track");
    assert_eq!(src.info().audio.as_ref().map(|a| a.channels), Some(3), "the sound holds the channels of both");
    let window = src.audio(12_000, 4_800, RATE).expect("the sound reads").channels;
    for (c, &freq) in freqs.iter().enumerate() {
        assert_tone(&window, c, freq, &freqs);
    }
}

/// Interlaced and 4:2:2 H.264 in Matroska is refused at open, saying why, as in MP4; the same
/// track coded progressive 8-bit 4:2:0 opens.
#[test]
fn interlaced_h264_in_matroska_is_refused_at_open() {
    let mkv = |sps: Vec<u8>| {
        let mut spec = TrackSpec::new(TrackKind::Video, "V_MPEG4/ISO/AVC");
        spec.codec_private = AvcConfig::new(vec![sps], vec![vec![0x68, 0xCE, 0x38, 0x80]], 4).to_bytes();
        spec.video_size = Some((16, 32));
        spec.default_duration_ns = Some(40_000_000);
        matroska(vec![spec], &[(0, 0, vec![0, 0, 0, 2, 0x65, 0x88])])
    };
    MkvSource::open("progressive.mkv", mkv(sps(100, 1, 8, None))).expect("progressive 8-bit 4:2:0 H.264 opens");
    for (name, sps, why) in [("mbaff.mkv", sps(100, 1, 8, Some(true)), "interlaced"), ("high-422-10.mkv", sps(122, 2, 10, None), "4:2:2")] {
        match MkvSource::open(name, mkv(sps)) {
            Err(CodecError::Unsupported(message)) => assert!(message.contains(why), "{name}: the refusal says {why}: {message}"),
            Err(other) => panic!("{name} is refused as unsupported, not {other:?}"),
            Ok(_) => panic!("{name} is refused at open"),
        }
    }
}

/// H.264 in MXF that the decoder does not implement used to open and decode to black frames, with
/// concealment counts that meant nothing: 10-bit 4:2:2 AVC-Intra without parameter sets in the
/// stream (the picture descriptor says what it is), and interlaced coding the SPS of the first
/// picture says. The picture is refused, saying why (the import's warning), and the sound still
/// plays; a progressive 8-bit 4:2:0 picture is not refused.
#[test]
fn avc_intra_and_interlaced_h264_in_mxf_are_refused_and_the_sound_plays() {
    let avc = |profile_idc| PictureDesc::new(PictureCoding::Avc { profile_idc, intra: true }, 16, 16);
    let progressive = MxfSource::open("progressive.mxf", avc_mxf(&idr_access_unit(&sps(100, 1, 8, None), 0, 128), avc(100))).expect("the file opens");
    assert!(progressive.warnings().is_empty(), "progressive 8-bit 4:2:0 H.264 is not refused: {:?}", progressive.warnings());
    let mut avc_intra = avc(122);
    avc_intra.depth = 10;
    avc_intra.subsampling = (2, 1);
    let cases = [
        ("avc-intra.mxf", avc_mxf(&annexb(5, &[0x88, 0x80]), avc_intra), ["without parameter sets", "4:2:2", "10-bit"]),
        ("mbaff.mxf", avc_mxf(&idr_access_unit(&sps(100, 1, 8, Some(true)), 0, 128), avc(100)), ["interlaced", "MBAFF", "progressive"]),
    ];
    for (name, bytes, reasons) in cases {
        let src = MxfSource::open(name, bytes).expect("the file opens, for its sound");
        let codec = src.info().video.as_ref().map(|v| v.codec.clone()).unwrap_or_default();
        assert!(codec.ends_with("(unsupported)"), "{name}: the picture is marked unsupported: {codec}");
        let warnings = src.warnings().join("; ");
        for why in reasons {
            assert!(warnings.contains(why), "{name}: the warning says {why}: {warnings}");
        }
        match src.video_frame(FrameRequest::full(Tick::ZERO)) {
            Err(MediaError::Unsupported(message)) => assert!(message.contains(reasons[0]), "{name}: {message}"),
            Err(other) => panic!("{name}: the picture is refused as unsupported, not {other:?}"),
            Ok(_) => panic!("{name}: the picture is refused, not decoded"),
        }
        let sound = src.audio(12_000, 4_800, RATE).expect("the sound plays").channels;
        assert_tone(&sound, 0, 440.0, &[440.0, 1000.0]);
    }
}

/// An off-air capture whose picture stops after a second while its sound runs on for three, with
/// two stereo sound streams (two languages). The sound holds the channels of both streams, each
/// its own track, and the capture lasts as long as its sound, so the sound after the picture's end
/// can be reached: a transport stream used to read one audio stream and last as long as its
/// picture.
#[test]
fn a_transport_stream_reads_every_audio_stream_and_lasts_as_long_as_its_sound() {
    let freqs = [440.0, 660.0, 880.0, 990.0];
    let picture = idr_access_unit(&baseline_sps(), 0, 128);
    let src = MpegSource::open("capture.ts", capture(&picture, 25, 3, freqs)).expect("the capture opens");
    assert_eq!(src.info().video.as_ref().map(|v| (v.width, v.height)), Some((16, 16)), "the picture is the capture's");
    assert_eq!(src.audio_tracks(), vec![2, 2], "two stereo streams");
    assert_eq!(src.info().audio.as_ref().map(|a| a.channels), Some(4), "the sound holds the channels of both");
    assert_eq!(src.info().duration, Tick(3 * TICKS_PER_SECOND), "the capture lasts as long as its sound");
    let late = src.audio(i64::from(RATE) * 5 / 2, 4_800, RATE).expect("the sound after the picture's end reads").channels;
    for (c, &freq) in freqs.iter().enumerate() {
        assert_tone(&late, c, freq, &freqs);
    }
    assert!(src.warnings().is_empty(), "{:?}", src.warnings());
}

/// Interlaced H.264 in a transport stream (a 1080i broadcast capture) used to open and decode to
/// black frames, or not open; it is refused as an unknown picture coding is, saying why, and the
/// sound still plays for the length of the capture.
#[test]
fn interlaced_h264_in_a_transport_stream_is_refused_and_the_sound_plays() {
    let freqs = [440.0, 660.0, 880.0, 990.0];
    let picture = idr_access_unit(&sps(100, 1, 8, Some(true)), 0, 128);
    let src = MpegSource::open("interlaced.ts", capture(&picture, 25, 2, freqs)).expect("the capture opens, for its sound");
    let codec = src.info().video.as_ref().map(|v| v.codec.clone()).unwrap_or_default();
    assert_eq!(codec, "H.264 (unsupported)", "the picture is marked unsupported");
    let warnings = src.warnings().join("; ");
    assert!(warnings.contains("interlaced"), "the import says why: {warnings}");
    assert!(matches!(src.video_frame(FrameRequest::full(Tick::ZERO)), Err(MediaError::Unsupported(_))), "the picture is refused, not decoded");
    assert_eq!(src.info().duration, Tick(2 * TICKS_PER_SECOND), "the capture lasts as long as its sound");
    let sound = src.audio(i64::from(RATE), 4_800, RATE).expect("the sound plays").channels;
    for (c, &freq) in freqs.iter().enumerate() {
        assert_tone(&sound, c, freq, &freqs);
    }
}
