//! How a clip plays a source whose sound holds several audio tracks (batch BF-2 of the LTS editor
//! bug fixes, OC-18; master plan v3 W7). A source now reads the channels of every audio track of
//! its file, one track after the other, and says how many each track holds
//! (`MediaSource::audio_tracks`). With no channels picked, a clip on a stereo track plays the
//! first track as that track is laid out, never the first channels of the next track: a 5.1 track
//! as its ITU-R BS.775 downmix, a stereo track as it is, a mono track on both sides.
//!
//! | Test | What it pins |
//! |---|---|
//! | `a_51_track_before_a_stereo_track_plays_its_downmix` | 5.1 then stereo: the 5.1's downmix, with its centre |
//! | `six_mono_tracks_are_not_a_51_track` | six mono inputs: the first on both sides, never a downmix |
//! | `two_mono_tracks_are_not_left_and_right` | two mono inputs: the first on both sides, not hard left and right |
//! | `one_track_of_six_channels_is_a_51_track` | one six-channel track (a 5.1 WAV): its downmix, as before |
//!
//! The sources are tones made here; nothing is read from disk.

use std::sync::Arc;

use filmcraft_frame::{AudioBuffer, VideoFrame};
use filmcraft_media::{AudioStreamInfo, FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource};
use filmcraft_project::{ItemKind, Label, MediaClip, MediaRef, Project, SequenceSettings, TrackKind};
use filmcraft_render::SourceMap;
use filmcraft_render::audio::mix_sequence;
use filmcraft_time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};

/// The sample rate of the sources and of the mix.
const RATE: u32 = 48_000;

/// A source whose channels each carry a tone of `freqs`, laid out as `tracks` (the channel count of
/// each of its audio tracks, in order).
struct Tones {
    info: MediaInfo,
    freqs: Vec<f64>,
    tracks: Vec<u32>,
}

impl MediaSource for Tones {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn video_frame(&self, _: FrameRequest) -> filmcraft_media::Result<Arc<VideoFrame>> {
        Err(MediaError::Unsupported("sound only".into()))
    }

    fn audio(&self, start: i64, frames: usize, sample_rate: u32) -> filmcraft_media::Result<AudioBuffer> {
        let at = |freq: f64, i: usize| (0.2 * (2.0 * std::f64::consts::PI * freq * (start + i as i64) as f64 / f64::from(sample_rate)).sin()) as f32;
        Ok(AudioBuffer { sample_rate, channels: self.freqs.iter().map(|&freq| (0..frames).map(|i| at(freq, i)).collect()).collect() })
    }

    fn audio_tracks(&self) -> Vec<u32> {
        self.tracks.clone()
    }
}

/// The stereo mix, from sample 12 000 for 4 800 samples, of a sequence that holds a second of a
/// [`Tones`] source on its one stereo audio track, with no channels picked.
fn mix(freqs: &[f64], tracks: &[u32]) -> Vec<Vec<f32>> {
    let mut p = Project::new("layouts");
    let seq = p.new_sequence("s", SequenceSettings::default(), 0, 1, None);
    let info = MediaInfo {
        name: "tones".into(),
        kind: MediaKind::AudioOnly,
        duration: Tick(TICKS_PER_SECOND),
        video: None,
        audio: Some(AudioStreamInfo { sample_rate: RATE, channels: freqs.len() as u32, codec: "test".into(), bits_per_sample: Some(32) }),
        container: "test".into(),
        start_timecode: None,
        file_size: None,
    };
    let id = p.add_item(
        "tones",
        Label::Iris,
        ItemKind::Media(MediaClip {
            media: MediaRef::File { path: "/tones.wav".into() },
            info: info.clone(),
            interpret: Default::default(),
            mark_in: None,
            mark_out: None,
            markers: vec![],
            offline: false,
            proxy: None,
            identity: None,
        }),
        None,
    );
    let clip = p.make_track_item(id, TrackKind::Audio, Tick::ZERO, TimeRange::new(Tick::ZERO, Tick(TICKS_PER_SECOND)), FrameRate::FPS_25).expect("a clip");
    p.sequence_mut(seq).expect("the sequence").audio_tracks[0].items.push(clip);
    let mut map = SourceMap::default();
    map.0.insert(id, Arc::new(Tones { info, freqs: freqs.to_vec(), tracks: tracks.to_vec() }));
    mix_sequence(&p, p.sequence(seq).expect("the sequence"), 12_000, 4_800, &map).channels
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

/// Checks that each side of `stereo` carries the tones its `present` list names, each well above
/// the noise, and none of its `absent` list.
fn assert_sides(stereo: &[Vec<f32>], freqs: &[f64], sides: [(&[f64], &[f64]); 2]) {
    for (side, (present, absent)) in sides.iter().enumerate() {
        let x = &stereo[side];
        let top = freqs.iter().map(|&f| tone_power(x, f)).fold(0.0, f64::max);
        assert!(top > 0.0, "side {side} carries sound");
        for &freq in *present {
            assert!(tone_power(x, freq) > top * 0.05, "side {side} carries the {freq} Hz channel");
        }
        for &freq in *absent {
            assert!(tone_power(x, freq) < top * 0.001, "side {side} carries no {freq} Hz");
        }
    }
}

/// A 5.1 track (L, R, C, LFE, Ls, Rs) and a stereo track after it: eight channels in all. The clip
/// plays the 5.1 track's BS.775 downmix, with its centre and its surrounds and without its LFE, and
/// nothing of the stereo track; it used to play the first two channels, without the centre.
#[test]
fn a_51_track_before_a_stereo_track_plays_its_downmix() {
    let freqs = [300.0, 500.0, 1000.0, 80.0, 700.0, 900.0, 1500.0, 1700.0];
    let stereo = mix(&freqs, &[6, 2]);
    let left: (&[f64], &[f64]) = (&[300.0, 1000.0, 700.0], &[500.0, 80.0, 900.0, 1500.0, 1700.0]);
    let right: (&[f64], &[f64]) = (&[500.0, 1000.0, 900.0], &[300.0, 80.0, 700.0, 1500.0, 1700.0]);
    assert_sides(&stereo, &freqs, [left, right]);
}

/// Six mono tracks (the inputs of a recorder) are six tracks of one channel: the clip plays the
/// first on both sides, and never folds the six as if they were a 5.1 track.
#[test]
fn six_mono_tracks_are_not_a_51_track() {
    let freqs = [300.0, 500.0, 700.0, 900.0, 1100.0, 1300.0];
    let stereo = mix(&freqs, &[1; 6]);
    let first: (&[f64], &[f64]) = (&[300.0], &[500.0, 700.0, 900.0, 1100.0, 1300.0]);
    assert_sides(&stereo, &freqs, [first, first]);
}

/// Two mono tracks (two microphones) play the first on both sides, as the file played before every
/// track was read; they used to play hard left and hard right.
#[test]
fn two_mono_tracks_are_not_left_and_right() {
    let freqs = [300.0, 500.0];
    let stereo = mix(&freqs, &[1, 1]);
    let first: (&[f64], &[f64]) = (&[300.0], &[500.0]);
    assert_sides(&stereo, &freqs, [first, first]);
}

/// One track of six channels (a 5.1 WAV, an AAC 5.1 track) is still played as its BS.775 downmix.
#[test]
fn one_track_of_six_channels_is_a_51_track() {
    let freqs = [300.0, 500.0, 1000.0, 80.0, 700.0, 900.0];
    let stereo = mix(&freqs, &[6]);
    let left: (&[f64], &[f64]) = (&[300.0, 1000.0, 700.0], &[500.0, 80.0, 900.0]);
    let right: (&[f64], &[f64]) = (&[500.0, 1000.0, 900.0], &[300.0, 80.0, 700.0]);
    assert_sides(&stereo, &freqs, [left, right]);
}
