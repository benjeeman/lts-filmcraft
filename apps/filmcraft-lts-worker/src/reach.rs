//! Native reach (slice RN, 2026-10-23): what the engine already does that a native session could
//! not reach, for vertical shorts cut from a broadcast episode (an 8-12 GB MXF that only a native
//! worker opens).
//!
//! - [`precheck`]: the table's native-only entries checked against the session before they run, so
//!   that a call the engine would fail from inside the command (which settles `unknown` and retires a
//!   native session) is refused `invalid_params` with nothing run. Modify Audio Channels and Breakout
//!   to Mono name media items with sound (the engine would otherwise fall back to the clip selection)
//!   and only channels those items have; a timecode edit names a media item; a frame size stays within
//!   the engine's pixel budget; a Crop or Auto Reframe parameter names a clip of the active sequence
//!   that holds that effect; a sequence opened as the active one is a sequence of the project.
//! - `importCaptions {requestId, handle, offsetTicks?, dropBeforeOffset?, windowTicks?, name?, expect,
//!   deadlineMs}` ([`Worker::import_captions`]): a caption file staged by handle, imported by the
//!   engine's own reader (`captions.import`'s, run on the workspace file: no path crosses the
//!   protocol) as a caption track of the active sequence, never of a sequence it would make. The
//!   file's time `offsetTicks` (a whole number of the sequence's frames) lands on the sequence's first
//!   frame: the engine imports at the file's own times, snapped to the sequence's frames, and the
//!   worker then deletes the cues the call drops (those before the offset, which are refused unless
//!   the call drops them, and those from the end of the window on) and moves the rest by the offset,
//!   so every kept cue lands exactly where it would have with no offset in the file. One request,
//!   one undo step (`Import Captions …`).
//! - [`media`]: the `media {item, levels?: {start, duration}}` read: what the engine read of an
//!   item's file (its container, codec and profile, whether a decoder takes the coding, the pixel
//!   format and the scan its flags declare, the frame count its duration makes and whether it is
//!   exact, its start timecode, its sound and how the sound's channels lie over the file's tracks,
//!   the channel map the item plays, its colour) and, asked for a window of the item's own time, each
//!   source channel's peak and RMS level. The scan is the first picture's coding flags as the file
//!   declares them: a flag, not an analysis of the pictures (progressive content carried as
//!   interlaced, PsF, is flagged interlaced).
//! - [`watermark`]: the `watermark` read: what binds a typed edit as `executeTyped` checks it (the
//!   revision, the active sequence, the history head, linked selection and the track targeting
//!   digest), so a host can bind undo, redo and the edits of the targeted tracks.
//! - [`Sidecar`] and [`sidecar_file`]: an export's caption sidecar (`captionSidecar: srt | vtt`,
//!   `rtlLineMarks: none | auto`), the first caption track of the sequence for the span the export
//!   renders (the cues before it dropped, the rest shifted to its start and cut at its end), written
//!   by the engine's own caption writer, which starts each right-to-left line with U+200F: a sidecar
//!   with such a line is `not_supported` unless the call asks `auto`.

use std::borrow::Cow;

use filmcraft_engine::Services;
use filmcraft_engine::Session;
use filmcraft_engine::export::ExportSettings;
use filmcraft_engine::project::{ItemId, MediaClip, MediaRef, Project, TrackId};
use filmcraft_engine::time::{FrameRate, TICKS_PER_SECOND, Tick};
use filmcraft_media::{AudioStreamInfo, MediaInfo, MediaSource, SharedSource, VideoStreamInfo};
use serde_json::{Map, Value, json};

use crate::agent_view_policy as policy;
use crate::reads::Answer;
use crate::services::ContainedServices;
use crate::{Worker, deadline_of, error, expectation, observe, preconditions, refused_why, request_id};

/// The effects a native worker adds and reaches beyond the intrinsic ones (the table's native
/// targets): a call names them only on a clip that holds them.
const ADDED_EFFECTS: [&str; 2] = ["crop", "auto_reframe"];

/// The commands that set or keyframe an effect parameter.
const PARAMETER_COMMANDS: [&str; 5] = ["effects.setParam", "effects.addKeyframe", "effects.setKeyframe", "effects.setInterpolation", "effects.deleteKeyframe"];

/// The longest window a levels read measures, in seconds.
const MAX_LEVEL_SECONDS: i64 = 600;

/// The sample frames a levels read decodes at a time.
const LEVEL_CHUNK: usize = 48_000;

/// The peak under which a channel reads as silent (-120 dBFS).
const SILENT: f64 = 1e-6;

/// A refusal of a read or a job before it ran: a registered code and why.
type Refused = (&'static str, String);

/// The pixel formats' endings that say how a picture is scanned, as the engine names them.
const UPPER_FIELD_FIRST: &str = "interlaced (upper field first)";
const LOWER_FIELD_FIRST: &str = "interlaced (lower field first)";
const PROGRESSIVE: &str = ", progressive";

/// What a scan reported from the file's flags rests on, and what a scan the editor does not report rests on.
const FLAGGED: &str = "the first picture's coding flags as the file declares them: a flag, not a look at the pictures (PsF is flagged interlaced)";
const UNFLAGGED: &str = "the editor reports no field flag for this coding";

/// Why a call the table admits would fail inside the engine, checked before it runs; `Ok` lets it through.
pub(crate) fn precheck(session: &Session, command: &str, params: &Value) -> Result<(), String> {
    match command {
        "clip.audioChannels" | "clip.breakoutToMono" => sound_items(session, params),
        "clip.modifyTimecode" => media_item(session, params.get("item").unwrap_or(&Value::Null)).map(|_| ()),
        "sequence.settings" => frame_size(params),
        "sequence.open" => sequence_item(session, params),
        _ if PARAMETER_COMMANDS.contains(&command) => effect_on_clip(session, params),
        _ => Ok(()),
    }
}

/// The media item `id` names: a media item of the project (a subclip or a sequence is none).
fn media_item<'a>(session: &'a Session, id: &Value) -> Result<&'a MediaClip, String> {
    let item = id.as_u64().ok_or_else(|| "an item is named by its id".to_string())?;
    session.project.item(ItemId(item)).and_then(|it| it.as_media()).ok_or_else(|| format!("item {item} is no media item of the project"))
}

/// Every item `items` names is a media item with sound, and every channel `clips` picks is one of each item's.
fn sound_items(session: &Session, params: &Value) -> Result<(), String> {
    let items = params.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
    let clips = params.get("clips").and_then(Value::as_array);
    for id in &items {
        let media = media_item(session, id)?;
        let channels = media.info.audio.as_ref().map(|audio| audio.channels).ok_or_else(|| format!("item {id} has no sound"))?;
        let mut picked = clips.into_iter().flatten().filter_map(Value::as_array).flatten().filter_map(Value::as_u64);
        if let Some(channel) = picked.find(|&channel| channel >= u64::from(channels)) {
            return Err(format!("item {id} has {channels} channels, counted from 0: it has no channel {channel}"));
        }
    }
    Ok(())
}

/// The item a sequence is opened by is a sequence of the project.
fn sequence_item(session: &Session, params: &Value) -> Result<(), String> {
    let item = params.get("item").and_then(Value::as_u64).ok_or_else(|| "a sequence is opened by its item's id".to_string())?;
    session.project.sequence(ItemId(item)).map(|_| ()).ok_or_else(|| format!("item {item} is no sequence of the project"))
}

/// A frame size the engine takes: within its longest side and its pixel budget.
fn frame_size(params: &Value) -> Result<(), String> {
    let side = |key: &str| params.get(key).and_then(Value::as_u64).and_then(|side| u32::try_from(side).ok());
    match (side("width"), side("height")) {
        (Some(width), Some(height)) => filmcraft_engine::project::validate_frame_size(width, height),
        _ => Ok(()),
    }
}

/// A Crop or Auto Reframe parameter names a clip of the active sequence's video tracks that holds that effect.
fn effect_on_clip(session: &Session, params: &Value) -> Result<(), String> {
    let effect = params.get("effect").and_then(Value::as_str).unwrap_or_default();
    if !ADDED_EFFECTS.contains(&effect) {
        return Ok(());
    }
    let clip = params.get("clip").and_then(Value::as_u64).unwrap_or(u64::MAX);
    let sequence = session.active_sequence().ok_or_else(|| "no sequence is open".to_string())?;
    let found = sequence.video_tracks.iter().flat_map(|track| track.items.iter()).find(|item| item.id.0 == clip);
    let found = found.ok_or_else(|| format!("clip {clip} is no clip of the active sequence's video tracks"))?;
    if !found.effects.iter().any(|instance| instance.effect == effect) {
        return Err(format!("clip {clip} holds no {effect}: add it with effects.apply first"));
    }
    Ok(())
}

/// The `watermark` read: the revision, the active sequence, the history head, linked selection and the track targeting
/// digest, as `executeTyped` checks a caller's `expect` against them.
pub(crate) fn watermark(session: &Session) -> Value {
    let observed = observe(session);
    let head = &observed.history_head;
    json!({
        "revision": observed.revision.to_string(),
        "activeSequence": observed.active_sequence.map(|id| id.to_string()),
        "historyHead": {"length": head.length, "label": head.label},
        "linkedSelection": observed.linked_selection,
        "targeting": observed.targeting,
    })
}

/// The `media {item, levels?}` read (see the module documentation).
pub(crate) fn media(worker: &mut Worker, params: &Value) -> Answer {
    let item = params.get("item").and_then(Value::as_u64).ok_or_else(|| ("invalid_params", "`item` is a media item's id".to_string()))?;
    let id = ItemId(item);
    let media = worker.session.project.item(id).and_then(|it| it.as_media()).cloned();
    let media = media.ok_or_else(|| ("target_not_found", format!("no media item {item}")))?;
    let window = levels_window(params.get("levels"))?;
    let session = &worker.session;
    let source = session.media.cached(id).or_else(|| session.media.full_res_source(&session.project, id, &*worker.services));
    let tracks = match worker.layouts.get(&item) {
        Some(tracks) => tracks.clone(),
        None => {
            let tracks = layout(&worker.services, &media, source.as_ref());
            worker.layouts.insert(item, tracks.clone());
            tracks
        }
    };
    let info = &media.info;
    let mut value = json!({
        "item": item,
        "kind": info.kind,
        "container": info.container,
        "durationTicks": info.duration.0.to_string(),
        "video": info.video.as_ref().map(|video| video_of(video, info.duration)),
        "audio": info.audio.as_ref().map(|audio| audio_of(audio, &tracks, &media)),
        "startTimecode": info.start_timecode.map(|frames| timecode_of(frames, info.frame_rate())),
        "warnings": source.as_ref().map(|source| source.warnings()).unwrap_or_default(),
        "damage": source.as_ref().map(|source| source.damage()).unwrap_or_default(),
    });
    if let Ok(color) = worker.session.execute("media.colorInfo", json!({"item": item})) {
        value["color"] = color;
    }
    if let Some((start, duration)) = window {
        let source = source.ok_or_else(|| ("read_failed", format!("the file of item {item} does not open")))?;
        value["levels"] = levels(source.as_ref(), info, start, duration)?;
    }
    Ok((value, Vec::new()))
}

/// The window `{start, duration}` of a levels read, in ticks of the item's own time (decimal strings, or whole
/// numbers): none when the read asks none.
fn levels_window(asked: Option<&Value>) -> Result<Option<(i64, i64)>, Refused> {
    let Some(asked) = asked else { return Ok(None) };
    let tick = |key: &str| asked.get(key).and_then(|value| value.as_str().and_then(|text| text.parse::<i64>().ok()).or_else(|| value.as_i64()));
    let shape = || ("invalid_params", "`levels` is {start, duration}: ticks of the item's own time, from 0, as decimal strings".to_string());
    let (Some(start), Some(duration)) = (tick("start"), tick("duration")) else { return Err(shape()) };
    if start < 0 || duration <= 0 || start > policy::MAX_TICKS {
        return Err(shape());
    }
    if duration > MAX_LEVEL_SECONDS * TICKS_PER_SECOND {
        return Err(("budget_exceeded", format!("a levels read measures at most {MAX_LEVEL_SECONDS} s at a time")));
    }
    Ok(Some((start, duration)))
}

/// The channel count of each track of the item's file whose channels its sound holds, in the order it holds them: an
/// MXF's sound tracks as the engine combines them (it opens the file again, once per item and worker), else what the
/// open source reports.
fn layout(services: &ContainedServices, media: &MediaClip, source: Option<&SharedSource>) -> Vec<u32> {
    let reported = || source.map(|source| source.audio_tracks()).unwrap_or_default();
    let MediaRef::File { path } = &media.media else { return reported() };
    if !media.info.container.starts_with("MXF") {
        return reported();
    }
    let reader = services.reader(path).and_then(Result::ok);
    let opened = reader.and_then(|reader| filmcraft_codecs::MxfSource::open_reader(&media.info.name, reader).ok());
    opened.map(|mxf| mxf.sound_tracks()).unwrap_or_else(reported)
}

/// What the engine read of a picture: its size, rate, aspect, codec and profile (the coding the codec's name gives in
/// brackets), whether a decoder takes it, its pixel format, bitrate and scan, and the frames its duration makes.
fn video_of(video: &VideoStreamInfo, duration: Tick) -> Value {
    let rate = video.frame_rate;
    let count = rate.frame_at(duration);
    let unsupported = video.codec.ends_with("(unsupported)");
    let profile = video.codec.strip_suffix(')').and_then(|codec| codec.split_once(" (")).map(|(_, profile)| profile).filter(|_| !unsupported);
    json!({
        "width": video.width,
        "height": video.height,
        "rate": {"num": rate.num, "den": rate.den},
        "par": [video.par.0, video.par.1],
        "codec": video.codec,
        "profile": profile,
        "decodable": !unsupported,
        "pixelFormat": video.pixel_format,
        "bitrate": video.bitrate,
        "scan": scan_of(&video.pixel_format),
        "frames": {"count": count, "exact": rate.tick_of(count) == duration},
    })
}

/// The scan a pixel format names: interlaced with its field order, progressive, or unknown, and what that rests on.
fn scan_of(pixel_format: &str) -> Value {
    let (flag, order) = match pixel_format {
        format if format.ends_with(UPPER_FIELD_FIRST) => ("interlaced", Some("tff")),
        format if format.ends_with(LOWER_FIELD_FIRST) => ("interlaced", Some("bff")),
        format if format.ends_with(PROGRESSIVE) => ("progressive", None),
        _ => ("unknown", None),
    };
    let basis = if flag == "unknown" { UNFLAGGED } else { FLAGGED };
    json!({"flag": flag, "fieldOrder": order, "basis": basis})
}

/// What the engine read of the sound: its rate, channels, codec and bit depth, its tracks (each with its channel count
/// and the sound's channel it starts at) and the channel map the item plays (none: the first two channels).
fn audio_of(audio: &AudioStreamInfo, tracks: &[u32], media: &MediaClip) -> Value {
    let mut first = 0u32;
    let mut listed = Vec::new();
    for (index, &channels) in tracks.iter().enumerate() {
        listed.push(json!({"track": index + 1, "channels": channels, "firstChannel": first}));
        first += channels;
    }
    json!({
        "sampleRate": audio.sample_rate,
        "channels": audio.channels,
        "codec": audio.codec,
        "bitsPerSample": audio.bits_per_sample,
        "tracks": listed,
        "channelMap": media.interpret.audio_channels,
    })
}

/// A start timecode of `frames` at `rate`: the frames, the timecode at the rate's timecode base (non-drop), and its
/// time in ticks.
fn timecode_of(frames: i64, rate: FrameRate) -> Value {
    let base = rate.timecode_base();
    let total = frames.max(0);
    let (frame, seconds) = (total % base, total / base);
    let text = format!("{:02}:{:02}:{:02}:{frame:02}", seconds / 3600, seconds / 60 % 60, seconds % 60);
    json!({"frames": frames, "timecode": text, "dropFrame": false, "tick": rate.tick_of(frames).0.to_string(), "rate": {"num": rate.num, "den": rate.den}})
}

/// The sample frames at `rate` of `ticks`.
fn to_samples(ticks: i64, rate: u32) -> usize {
    let scaled = i128::from(ticks.max(0)) * i128::from(rate) / i128::from(TICKS_PER_SECOND);
    usize::try_from(scaled).unwrap_or(usize::MAX)
}

/// Each source channel's peak and RMS level over `[start, start + duration)` ticks of the item's own time, decoded a
/// second at a time; a channel whose peak is under -120 dBFS is silent and has no level.
fn levels(source: &dyn MediaSource, info: &MediaInfo, start: i64, duration: i64) -> Result<Value, Refused> {
    let audio = info.audio.as_ref().ok_or_else(|| ("invalid_params", "the item has no sound".to_string()))?;
    let rate = audio.sample_rate.max(1);
    let (first, frames) = (to_samples(start, rate), to_samples(duration, rate));
    let channels = audio.channels as usize;
    let (mut peaks, mut squares) = (vec![0f64; channels], vec![0f64; channels]);
    let mut done = 0usize;
    while done < frames {
        let take = LEVEL_CHUNK.min(frames - done);
        let at = i64::try_from(first + done).unwrap_or(i64::MAX);
        let buffer = source.audio(at, take, rate).map_err(|e| ("read_failed", e.to_string()))?;
        for (channel, samples) in buffer.channels.iter().enumerate().take(channels) {
            for &sample in samples.iter().take(take) {
                let sample = f64::from(sample);
                peaks[channel] = peaks[channel].max(sample.abs());
                squares[channel] += sample * sample;
            }
        }
        done += take;
    }
    let mut listed = Vec::new();
    for channel in 0..channels {
        let silent = peaks[channel] < SILENT;
        let db = |level: f64| (!silent).then(|| 20.0 * level.log10());
        let rms = (squares[channel] / frames.max(1) as f64).sqrt();
        listed.push(json!({"channel": channel, "peakDbfs": db(peaks[channel]), "rmsDbfs": db(rms), "silent": silent}));
    }
    Ok(json!({"startTick": start.to_string(), "durationTicks": duration.to_string(), "sampleRate": rate, "frames": frames, "channels": listed}))
}

/// What an `importCaptions` message asks.
struct CaptionImport {
    handle: String,
    offset: i64,
    drop_before: bool,
    window: Option<i64>,
    name: Option<String>,
}

impl CaptionImport {
    /// The import an `importCaptions` message asks, or why its keys do not read.
    fn of(message: &Value) -> Result<CaptionImport, String> {
        let ticks = |key: &str| -> Result<Option<i64>, String> {
            let wrong = || format!("`{key}` is ticks as a decimal string");
            match message.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(text)) => text.parse::<i64>().map(Some).map_err(|_| wrong()),
                Some(other) => other.as_i64().map(Some).ok_or_else(wrong),
            }
        };
        let offset = ticks("offsetTicks")?.unwrap_or(0);
        let window = ticks("windowTicks")?;
        if !(0..=policy::MAX_TICKS).contains(&offset) || window.is_some_and(|window| !(1..=policy::MAX_TICKS).contains(&window)) {
            return Err("`offsetTicks` is from 0 and `windowTicks` over 0, each within the engine's timeline".to_string());
        }
        let drop_before = match message.get("dropBeforeOffset") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(drop)) => *drop,
            Some(_) => return Err("`dropBeforeOffset` is true or false".to_string()),
        };
        let name = match message.get("name") {
            None | Some(Value::Null) => None,
            Some(Value::String(name)) if !name.trim().is_empty() => Some(name.clone()),
            Some(_) => return Err("`name` is a track name with more than white space in it".to_string()),
        };
        let handle = message.get("handle").and_then(Value::as_str).unwrap_or_default().to_string();
        Ok(CaptionImport { handle, offset, drop_before, window, name })
    }
}

/// The cues of caption track `track` of the active sequence, by id: those that start before `offset`, those that start
/// at `end` or later, and the rest.
fn split_cues(session: &Session, track: u64, offset: i64, end: Option<i64>) -> (Vec<u64>, Vec<u64>, Vec<u64>) {
    let (mut early, mut late, mut kept) = (Vec::new(), Vec::new(), Vec::new());
    let cues = session.active_sequence().and_then(|sequence| sequence.caption_track(TrackId(track))).map(|track| track.captions.clone()).unwrap_or_default();
    for cue in cues {
        let start = cue.start.0;
        if start < offset {
            early.push(cue.id.0);
        } else if end.is_some_and(|end| start >= end) {
            late.push(cue.id.0);
        } else {
            kept.push(cue.id.0);
        }
    }
    (early, late, kept)
}

impl Worker {
    /// Answer an `importCaptions` message (see the module documentation), journaled as a mutation.
    pub(crate) fn import_captions(&mut self, message: &Value) -> Value {
        let Some(request) = request_id(message) else { return error(message, "invalid_params", "`requestId` must be a non-empty string") };
        let expect = message.get("expect").cloned();
        let deadline = deadline_of(message);
        let asked = CaptionImport::of(message);
        self.journaled(&request, "importCaptions", true, |worker| worker.captions_from(asked, expect.as_ref(), deadline))
    }

    /// The import of the caption file `asked` names into the active sequence, bound to the session as the caller read it.
    fn captions_from(&mut self, asked: Result<CaptionImport, String>, expect: Option<&Value>, deadline_ms: f64) -> Value {
        let observed = observe(&self.session);
        let revision = observed.revision;
        let expect = expectation(expect, deadline_ms, &observed);
        if let Err(code) = preconditions(&expect, &observed) {
            return policy::refused(code, revision);
        }
        let asked = match asked {
            Ok(asked) => asked,
            Err(why) => return refused_why("invalid_params", revision, &why),
        };
        let Some(rate) = self.session.active_sequence().map(|sequence| sequence.settings.frame_rate) else {
            return refused_why("sequence_not_active", revision, "captions are imported into the active sequence: open one first");
        };
        if rate.tick_of(rate.frame_at(Tick(asked.offset))) != Tick(asked.offset) {
            return refused_why("frame_misaligned", revision, "the offset falls between two frames of the sequence");
        }
        let Some(path) = self.services.staged_path(&asked.handle) else { return refused_why("source_not_staged", revision, "the handle names no staged file") };
        let bytes = match self.services.read_file(&path) {
            Ok(bytes) => bytes,
            Err(e) => return refused_why("staged_file_corrupt", revision, &e.to_string()),
        };
        let Some(format) = filmcraft_engine::captions::detect(&path, &bytes) else {
            return refused_why("format_unsupported", revision, "the editor reads SRT, WebVTT, SCC, MCC, EBU STL, TTML and DFXP caption files");
        };
        let mut document = match filmcraft_captions::parse(&bytes, format) {
            Ok(document) => document,
            Err(e) => return refused_why("format_unsupported", revision, &format!("the file does not read as {}: {e}", format.name())),
        };
        document.snap_to_frames(rate);
        let end = asked.window.map(|window| asked.offset.saturating_add(window));
        let early = document.cues.iter().filter(|cue| cue.start.0 < asked.offset).count();
        let late = document.cues.iter().filter(|cue| end.is_some_and(|end| cue.start.0 >= end)).count();
        if early > 0 && !asked.drop_before {
            let why = format!("{early} cues start before the offset: drop them (dropBeforeOffset) or give an earlier offset");
            return refused_why("invalid_params", revision, &why);
        }
        if document.cues.len() == early + late {
            return refused_why("invalid_params", revision, "no cue starts from the offset on, inside the window");
        }
        let undo = self.session.history.undo.len();
        let imported = filmcraft_engine::captions::import(&mut self.session, &path, &bytes, format, asked.name.as_deref());
        let mut result = match imported {
            Ok(result) => result,
            Err(e) if self.session.revision == revision => return refused_why("invalid_params", revision, &e.to_string()),
            Err(e) => return policy::settle("captions.import", &json!({}), revision, self.session.revision, None, policy::Answer::Failed(e.to_string())),
        };
        let track = result.get("track").and_then(Value::as_u64).unwrap_or(u64::MAX);
        let (early, late, kept) = split_cues(&self.session, track, asked.offset, end);
        let dropped: Vec<u64> = early.iter().chain(&late).copied().collect();
        let mut steps = Vec::new();
        if !dropped.is_empty() {
            steps.push(("captions.delete", json!({"captions": dropped})));
        }
        if asked.offset > 0 {
            steps.push(("captions.move", json!({"captions": kept, "delta": -asked.offset})));
        }
        for (command, params) in steps {
            match self.session.execute(command, params) {
                Ok(moved) if command == "captions.move" && moved.get("applied").and_then(Value::as_i64) != Some(-asked.offset) => {
                    let why = format!("the cues moved {moved} ticks, not by the offset");
                    return policy::settle("captions.import", &json!({}), revision, self.session.revision, None, policy::Answer::Failed(why));
                }
                Ok(_) => {}
                Err(e) => return policy::settle("captions.import", &json!({}), revision, self.session.revision, None, policy::Answer::Failed(e.to_string())),
            }
        }
        // the import, the cues it drops and the move are one step of the history, as the engine's own collapsed steps are
        let history = &mut self.session.history;
        if history.undo.len() > undo + 1 {
            let first = history.undo[undo].clone();
            history.undo.truncate(undo);
            history.undo.push(first);
        }
        let label = self.session.history.undo.last().map(|(label, _)| label.clone());
        result["captions"] = json!(kept.len());
        result["dropped"] = json!({"beforeOffset": early.len(), "afterWindow": late.len()});
        result["offsetTicks"] = json!(asked.offset.to_string());
        policy::settle("captions.import", &json!({}), revision, self.session.revision, label.as_deref(), policy::Answer::Ok(result))
    }
}

/// An export's caption sidecar: the format the first caption track is written in, its file's extension, and whether
/// right-to-left lines may be marked (`rtlLineMarks: auto`).
pub(crate) struct Sidecar {
    format: filmcraft_captions::Format,
    extension: &'static str,
    marks: bool,
}

impl Sidecar {
    /// The sidecar a job's parameters ask (`captionSidecar: srt | vtt`, `rtlLineMarks: none | auto`), taken out of them
    /// (the engine's own export would write its own sidecar, uncut at the range's end); none when they ask none.
    pub(crate) fn take(keys: &mut Map<String, Value>) -> Result<Option<Sidecar>, Refused> {
        let marks = match keys.remove("rtlLineMarks") {
            None => false,
            Some(Value::String(marks)) if marks == "none" => false,
            Some(Value::String(marks)) if marks == "auto" => true,
            Some(_) => return Err(("invalid_params", "`rtlLineMarks` is none or auto".to_string())),
        };
        let (format, extension) = match keys.remove("captionSidecar") {
            None if marks => return Err(("invalid_params", "`rtlLineMarks: auto` marks the lines of a `captionSidecar`".to_string())),
            None => return Ok(None),
            Some(Value::String(format)) if format == "srt" => (filmcraft_captions::Format::Srt, "srt"),
            Some(Value::String(format)) if format == "vtt" => (filmcraft_captions::Format::WebVtt, "vtt"),
            Some(_) => return Err(("invalid_params", "`captionSidecar` is srt or vtt".to_string())),
        };
        Ok(Some(Sidecar { format, extension, marks }))
    }
}

/// The caption sidecar of an export of `sequence` with `settings` (see the module documentation): its file name in the
/// job's folder and its bytes; refused before the export runs when the sequence has no caption track (`invalid_params`)
/// or the writer would mark a right-to-left line the call did not let it mark (`not_supported`).
pub(crate) fn sidecar_file(project: &Project, sequence: ItemId, settings: &ExportSettings, sidecar: &Sidecar) -> Result<(String, Vec<u8>), Refused> {
    let q = project.sequence(sequence).ok_or_else(|| ("sequence_not_active", "no sequence is open".to_string()))?;
    let track = q.caption_tracks.first().ok_or_else(|| ("invalid_params", "the sequence has no caption track: a sidecar writes its first one".to_string()))?;
    let range = filmcraft_engine::export::export_range(project, sequence, settings).map_err(|e| ("invalid_params", e.to_string()))?;
    let mut document = filmcraft_captions::document_from_track(track, range.start);
    document.cues.retain(|cue| cue.start < range.duration);
    for cue in &mut document.cues {
        cue.end = cue.end.min(range.duration);
    }
    let marks = |line: &str| !line.trim().is_empty() && matches!(filmcraft_captions::mark_right_to_left(line), Cow::Owned(_));
    let marked = document.cues.iter().flat_map(|cue| cue.text.lines()).any(marks);
    if marked && !sidecar.marks {
        let why = "the editor's SRT and WebVTT writer starts each right-to-left line with U+200F: ask rtlLineMarks auto (RD-12), or leave the sidecar out";
        return Err(("not_supported", why.to_string()));
    }
    let options = filmcraft_captions::WriteOptions { drop_frame: q.settings.drop_frame, rate: Some(q.settings.frame_rate) };
    Ok((format!("export.{}", sidecar.extension), filmcraft_captions::write(&document, sidecar.format, options)))
}
