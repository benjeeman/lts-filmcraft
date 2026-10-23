//! The worker's reads (EWP `read`): frames of the active sequence from the engine's own compositor,
//! written as fresh images under `previews/` (T-NW-FR-01); the fonts the engine has; and the
//! certified queries of the hook's table. A read changes nothing the revision, the undo stack, the
//! playhead or the selection holds.
//!
//! A read with `deadlineMs` that passes is `read_timeout` (it is checked before each frame); a read
//! over the session's limits (`hello`'s `maxSide`, `maxSheetCells`, `maxPreviewBytes`) is
//! `budget_exceeded`. A frames read repeats the font set's digest, names the families the project's
//! text substitutes (RD-14), and marks every frame that shows media whose source concealed samples
//! that failed to decode, with a `partial` coverage for the read (media-agent-contracts.md §8.4).

use filmcraft_engine::project::Sequence;
use filmcraft_engine::time::Tick;
use filmcraft_engine::{EngineError, Session};
use filmcraft_media::Damage;
use serde_json::{Value, json};

use crate::agent_view_policy as policy;
use crate::{Worker, fonts, imaging, now_ms, services};

/// The read kinds a worker serves.
pub(crate) const KINDS: [&str; 3] = ["frames", "fonts", "query"];

/// The longest side of a frame a read renders, in pixels.
pub(crate) const MAX_SIDE: u32 = 4096;

/// The most frames one read renders.
pub(crate) const MAX_FRAMES: usize = 16;

/// What a read answers: its value and the files it wrote, or a registered code and why not.
type Answer = Result<(Value, Vec<Value>), (&'static str, String)>;

/// Answer a `read` message with its `readResult`.
pub(crate) fn read(worker: &mut Worker, message: &Value) -> Value {
    let read_id = message.get("readId").cloned().unwrap_or(Value::Null);
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
    // a read without a deadline has none
    let deadline_ms = message.get("deadlineMs").and_then(Value::as_f64).unwrap_or(f64::INFINITY);
    let answer = if now_ms() > deadline_ms {
        Err(("read_timeout", "the read's deadline passed before it ran".to_string()))
    } else {
        match message.get("kind").and_then(Value::as_str).unwrap_or_default() {
            "frames" => frames(worker, read_id.as_str().unwrap_or_default(), &params, deadline_ms),
            "fonts" => fonts_read(worker, &params),
            "query" => query(worker, &params),
            other => Err(("unsupported_read", format!("no read kind `{other}`"))),
        }
    };
    let mut result = match answer {
        Ok((value, files)) => json!({"type": "readResult", "readId": read_id, "ok": true, "value": value, "files": files}),
        Err((code, why)) => json!({"type": "readResult", "readId": read_id, "ok": false, "code": code, "message": why}),
    };
    result["consistency"] = json!("single-read");
    result["rev"] = json!(worker.session.revision.to_string());
    result
}

/// Why a frame did not render, as the hook's `renderFrame` says it.
fn render_failure(error: EngineError) -> (&'static str, String) {
    match error {
        EngineError::NoSequence => ("sequence_not_active", "no sequence is open".to_string()),
        error => ("read_failed", error.to_string()),
    }
}

/// A time in ticks: a whole number, or a decimal string of one.
fn ticks(value: &Value) -> Option<i64> {
    match value {
        Value::String(text) => text.parse().ok(),
        other => other.as_i64(),
    }
}

/// The media items whose clips show at `tick` on the enabled video tracks of `sequence` and whose
/// sources concealed samples so far (a sample that failed to decode shows the last good picture),
/// with what they concealed.
fn concealed_at(session: &Session, sequence: &Sequence, tick: Tick) -> Vec<(u64, Damage)> {
    let mut out: Vec<(u64, Damage)> = Vec::new();
    for track in sequence.video_tracks.iter().filter(|track| track.enabled) {
        for clip in track.items.iter().filter(|clip| clip.enabled && clip.start <= tick && tick < clip.end()) {
            let Some((item, _, _)) = session.project.resolve_media(clip.item) else { continue };
            let Some(damage) = session.media.cached(item).map(|source| source.damage()) else { continue };
            if damage.video_samples_concealed > 0 && !out.iter().any(|(id, _)| *id == item.0) {
                out.push((item.0, damage));
            }
        }
    }
    out
}

/// `frames {at: [{frame} | {tick}], maxSide, format: png | jpeg, quality?}`: each frame of the
/// active sequence as `try_render_program_at` renders it (over black, 8 bits) at the scale that fits
/// its long side in `maxSide`, written to `previews/<readId>-<k>.<ext>` (`readId` a plain name; the
/// k-th frame asked). The value names, for each frame, what was asked, the frame and tick it shows
/// and the media it shows that concealed samples (`concealed`); for the read, the font set's digest,
/// the families the project's text substitutes and, when a frame shows concealed media, a
/// `partial` coverage with what each source concealed.
fn frames(worker: &Worker, read_id: &str, params: &Value, deadline_ms: f64) -> Answer {
    let session = &worker.session;
    let limits = worker.limits;
    if !services::plain(read_id) {
        return Err(("invalid_params", "a frames read's readId is a plain name: letters, digits, `.`, `_` and `-`".to_string()));
    }
    let sequence = session.active_sequence().ok_or_else(|| ("sequence_not_active", "no sequence is open".to_string()))?;
    let max_side = params.get("maxSide").and_then(Value::as_u64).and_then(|side| u32::try_from(side).ok()).filter(|side| (1..=MAX_SIDE).contains(side));
    let max_side = max_side.ok_or_else(|| ("invalid_params", format!("`maxSide` is a whole number from 1 to {MAX_SIDE}")))?;
    if max_side > limits.max_side {
        return Err(("budget_exceeded", format!("`maxSide` {max_side} is over the session's {}", limits.max_side)));
    }
    let format = params.get("format").and_then(Value::as_str).unwrap_or("png");
    if !matches!(format, "png" | "jpeg") {
        return Err(("invalid_params", "`format` is png or jpeg".to_string()));
    }
    let quality = params.get("quality").and_then(Value::as_u64).map_or(85, |quality| quality.clamp(1, 100) as u8);
    let at = params.get("at").and_then(Value::as_array).filter(|at| !at.is_empty() && at.len() <= MAX_FRAMES);
    let at = at.ok_or_else(|| ("invalid_params", format!("`at` lists 1 to {MAX_FRAMES} frames")))?;
    if at.len() > limits.max_frames {
        return Err(("budget_exceeded", format!("{} frames are over the session's {} a read", at.len(), limits.max_frames)));
    }
    let rate = session.sequence_rate();
    let long_edge = sequence.settings.width.max(sequence.settings.height).max(1);
    let scale = (max_side as f32 / long_edge as f32).min(1.0);
    let substituted = fonts::substituted(&session.project);
    let (mut entries, mut files, mut damaged) = (Vec::new(), Vec::new(), Vec::new());
    for (k, wanted) in at.iter().enumerate() {
        if now_ms() > deadline_ms {
            return Err(("read_timeout", format!("the read's deadline passed after {k} of {} frames", at.len())));
        }
        let tick = match (wanted.get("frame").and_then(Value::as_i64), wanted.get("tick").and_then(ticks)) {
            (Some(frame), _) => rate.tick_of(frame),
            (None, Some(tick)) => Tick(tick),
            (None, None) => return Err(("invalid_params", "each entry of `at` names a `frame` or a `tick`".to_string())),
        };
        let tick = rate.snap(tick.max(Tick::ZERO));
        let image = session.try_render_program_at(scale, tick).map_err(render_failure)?;
        let (width, height) = (u32::try_from(image.w).unwrap_or(0), u32::try_from(image.h).unwrap_or(0));
        let rgba = image.over_black_rgba8();
        let encoded = if format == "jpeg" { imaging::encode_jpeg(width, height, &rgba, quality) } else { imaging::encode_png(width, height, &rgba) };
        let bytes = encoded.map_err(|why| ("read_failed", why))?;
        if let Some(cap) = limits.max_preview_bytes.filter(|&cap| bytes.len() as u64 > cap) {
            return Err(("budget_exceeded", format!("frame {k} is {} bytes, over the session's {cap} an image", bytes.len())));
        }
        let (mime, ext) = if format == "jpeg" { ("image/jpeg", "jpg") } else { ("image/png", "png") };
        let name = format!("{read_id}-{k}.{ext}");
        worker.services.write_in(&[services::PREVIEWS], &name, &bytes).map_err(|e| ("read_failed", e.to_string()))?;
        let actual = json!({"frame": rate.frame_at(tick), "tick": tick.0.to_string()});
        let mut entry = json!({"requested": wanted, "actual": actual, "width": width, "height": height, "file": files.len()});
        let concealed = concealed_at(session, sequence, tick);
        if !concealed.is_empty() {
            entry["concealed"] = json!(concealed.iter().map(|(item, _)| item).collect::<Vec<_>>());
        }
        for (item, damage) in concealed {
            if !damaged.iter().any(|(id, _)| *id == item) {
                damaged.push((item, damage));
            }
        }
        entries.push(entry);
        let (path, sha256) = (format!("{}/{name}", services::PREVIEWS), services::sha256_hex(&bytes));
        files.push(json!({"role": "frame", "path": path, "mime": mime, "bytes": bytes.len(), "sha256": sha256, "width": width, "height": height}));
    }
    let active = session.state.active_sequence.map(|id| id.0.to_string());
    let revision = session.revision.to_string();
    let mut value = json!({"frames": entries, "revision": revision, "activeSequence": active, "substituted": substituted});
    value["fontsetDigest"] = json!(worker.fonts.digest);
    if !damaged.is_empty() {
        let media: Vec<Value> = damaged.iter().map(|(item, damage)| json!({"item": item, "damage": damage})).collect();
        value["coverage"] = json!({"status": "partial", "why": "source_damaged", "media": media});
    }
    Ok((value, files))
}

/// `fonts {family?, text?}`: the font families the engine has, with their styles, the fonts `hello`
/// registered, the font set's digest and whether the system font folders were scanned (never, in a
/// worker: T-NW-ENV). With `family` and `text` (together), which characters of the text a render in
/// that family draws from no face, and which faces a fallback sets the others in (`coverage`).
fn fonts_read(worker: &Worker, params: &Value) -> Answer {
    let families: Vec<Value> = filmcraft_text::fonts::families().into_iter().map(|(family, styles)| json!({"family": family, "styles": styles})).collect();
    let scan = filmcraft_engine::graphics::system_scan_enabled();
    let registered = worker.fonts.listing();
    let mut value = json!({"families": families, "systemScanned": filmcraft_text::fonts::system_scanned(), "systemScan": scan, "registered": registered});
    value["fontsetDigest"] = json!(worker.fonts.digest);
    match (params.get("family"), params.get("text")) {
        (None, None) => {}
        (Some(Value::String(family)), Some(Value::String(text))) => value["coverage"] = fonts::coverage(family, text),
        _ => return Err(("invalid_params", "`family` and `text` come together, as strings".to_string())),
    }
    Ok((value, Vec::new()))
}

/// `query {command, params}`: one certified read of the hook's table ([`policy::READS`]) with the
/// keys it was certified with; every other command is `command_not_allowed`.
fn query(worker: &mut Worker, params: &Value) -> Answer {
    let command = params.get("command").and_then(Value::as_str).unwrap_or_default().to_string();
    let args = params.get("params").cloned().unwrap_or_else(|| json!({}));
    if !policy::READS.iter().any(|read| read.command == command) {
        return Err(("command_not_allowed", format!("`{command}` is not a certified read")));
    }
    policy::checked(&command, &args, policy::Runtime::HERE).map_err(|code| (code, format!("`{command}` with these parameters is refused")))?;
    let value = worker.session.execute(&command, args).map_err(|e| ("read_failed", e.to_string()))?;
    Ok((value, Vec::new()))
}
