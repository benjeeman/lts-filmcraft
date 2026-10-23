//! Target checks (wave native-acts, owner decisions OC-47 D1 and D11): what a typed edit names is checked against the
//! session it runs in, in the same call, before the engine runs anything. A target that is not there, or of another
//! kind than the act needs, is refused with the code the lab adapter refuses the same mistake with
//! (`creative-filmcraft-adapter.mjs`: `clipOf`, `trackOf`, `graphicOf`, `captionTrackIn`, `captionIn`, the subclip's
//! item and the marker edit), so the native and the lab runtime answer one agent mistake alike:
//!
//! - `target_not_found`: a clip, caption, caption track, marker, transition or item the session does not hold, and a
//!   clip that is no graphic for a graphic's edit; the agent names the target again from a fresh read;
//! - `track_missing`: a video or audio track, or a title's track, the active sequence does not have;
//! - `format_unsupported`: a target of another kind than the act needs: a sound's clip for a picture's act and the
//!   other way round, a clip moved to a track of the other kind, an item that is no media item for a subclip, a
//!   transcript or a speaker rename.
//!
//! The refusal is `not_applied` with that code, its `detail` naming the target and its `param` pointing at it in the
//! call's `params` (a JSON pointer, RFC 6901: `/clip`, `/clips/1`, `/moves/0/track`), and nothing has run: no
//! revision, no undo step and no Events line. Left to the engine, such a call fails inside the command, applies as an
//! undo step that changes nothing (a cut through no clip, a video effect for a sound's clip), applies to the targets
//! that are there and not to the others (an effect for two clips of which one is missing), commits a step before it
//! fails (a caption on a caption track that is not there, in a sequence that had none, after the engine made one), or
//! lands elsewhere (a shape for a clip that is no graphic starts a graphic clip of its own at the playhead, a title on
//! a video track past the last one takes a free track, a speaker renamed for an item that is no media item is renamed
//! in every transcript).
//!
//! In the active sequence:
//! - every clip a call names (`clip`, `clips`, each move's `clip`) is a clip of its video or audio tracks, of the kind
//!   the act needs: a picture's act (a video transition, Lumetri, fit and fill, a frame hold, scene detection, a mask,
//!   a video effect and its parameters and keyframes) a clip of a video track, a sound's act (an audio transition, an
//!   audio effect's parameters and keyframes) a clip of an audio track, and a graphic's edit a graphic clip;
//! - every video or audio track (`track` and `audioTrack`, by id or as `V1` and `A1` on, as the engine reads them; each
//!   move's `track`, of its clip's kind), caption track (by id or as `C1` on; the first caption of a sequence without a
//!   caption track may name `C1`, which the engine makes for it) and title track (by its index among the video
//!   tracks) is one of its own;
//! - every caption (`caption`, `captions`), the marker (`marker`: one of this sequence's own, as one session cuts
//!   several shorts, a sequence each) and the transition (`transition`) a call names are its own.
//!
//! An effect's act is a picture's or a sound's by the effect it names, looked up by its id. The table admits the
//! effect of each of these commands by its id only (`effects.apply` by its list of effects, the parameter and keyframe
//! commands by the effect parameters they reach), so a display name such as `Motion` is refused `command_not_allowed`
//! before any target is checked, and the engine finds a parameter's or a keyframe's effect on its clip by that id too.
//!
//! In the project: the item a place names, and the media item (or a subclip of one) a subclip is made of, a transcript
//! is set for or a speaker renamed in. The native-only entries have their own checks (`reach::precheck`). With no
//! sequence open, the engine refuses every command that needs one itself, before it runs (`command_disabled`), so
//! nothing is checked against a sequence then.

use filmcraft_engine::Session;
use filmcraft_engine::project::{ClipId, EffectKind, ItemId, Sequence, Track, TrackId, TrackKind, find_effect};
use serde_json::{Value, json};

use crate::agent_view_policy as policy;

/// The code of a target the session does not hold, as the lab adapter answers it.
const NOT_FOUND: &str = "target_not_found";

/// The code of a video or audio track, or a title's track, the active sequence does not have, as the lab adapter
/// answers it.
const NO_TRACK: &str = "track_missing";

/// The code of a target of another kind than the act needs, as the lab adapter answers it.
const OTHER_KIND: &str = "format_unsupported";

/// Why a call names a target the session does not hold, or one of another kind than its act needs.
#[derive(Debug)]
pub(crate) struct Miss {
    /// `target_not_found`, `track_missing` or `format_unsupported` (see the module documentation).
    code: &'static str,
    /// What is wrong, naming the target.
    detail: String,
    /// Where the call names the target: a JSON pointer into its `params` (RFC 6901).
    param: String,
}

impl Miss {
    /// A miss of `code` for the target the call names at `param`, `detail` saying what is wrong.
    fn new(code: &'static str, param: impl Into<String>, detail: String) -> Miss {
        Miss { code, detail, param: param.into() }
    }

    /// The refusal of the call, at `revision`: `not_applied` with the code, the `detail` and the `param`.
    pub(crate) fn refusal(&self, revision: u64) -> Value {
        let mut envelope = policy::refused(self.code, revision);
        envelope["detail"] = json!(self.detail);
        envelope["param"] = json!(self.param);
        envelope
    }
}

/// What an act needs of the clips it names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Needs {
    /// A clip of the sequence, of either kind.
    Clip,
    /// A clip of a video track.
    Picture,
    /// A clip of an audio track.
    Sound,
    /// A graphic clip.
    Graphic,
}

impl Needs {
    /// What an act of this kind takes, as a refusal says it.
    fn wants(self) -> &'static str {
        match self {
            Needs::Clip => "acts on a clip of the active sequence",
            Needs::Picture => "acts on a clip of a video track",
            Needs::Sound => "acts on a clip of an audio track",
            Needs::Graphic => "edits a graphic clip",
        }
    }
}

/// Why a call the table admits names a target the session does not hold, or one of another kind than its act needs;
/// `Ok` when every target it names is there (see the module documentation).
pub(crate) fn check(session: &Session, command: &str, params: &Value) -> Result<(), Miss> {
    item(session, command, params)?;
    let Some(sequence) = session.active_sequence() else { return Ok(()) };
    let call = Call { session, sequence, command, params };
    call.clips()?;
    call.tracks()?;
    call.captions()?;
    call.marker()?;
    call.transition()
}

/// The item a call names (`item`) is in the project: any item for a place; a media item, or a subclip of one, for a
/// subclip, a transcript set or a speaker renamed, where the engine has no media range for any other item (and reads an
/// item that is no media item as no item at all for a transcript, renaming the speaker in every transcript).
fn item(session: &Session, command: &str, params: &Value) -> Result<(), Miss> {
    let Some(id) = params.get("item").and_then(Value::as_u64) else { return Ok(()) };
    let media = match command {
        "timeline.place" => false,
        "clip.makeSubclip" | "transcript.set" | "transcript.renameSpeaker" => true,
        _ => return Ok(()),
    };
    let project = &session.project;
    if project.item(ItemId(id)).is_none() {
        return Err(Miss::new(NOT_FOUND, "/item", format!("item {id} is not in the project")));
    }
    if media && project.resolve_media(ItemId(id)).is_none() {
        return Err(Miss::new(OTHER_KIND, "/item", format!("item {id} is no media item, nor a subclip of one: `{command}` needs one")));
    }
    Ok(())
}

/// What the act of `command` needs of the clips it names (an effect's act: by the effect `params` name).
fn needs(command: &str, params: &Value) -> Needs {
    match command {
        "sequence.applyVideoTransition" | "lumetri.setSection" | "clip.fillFrame" | "clip.fitToFrame" | "clip.frameHold" => Needs::Picture,
        "clip.sceneEditDetection" | "masks.add" | "masks.set" | "masks.translate" | "masks.remove" | "masks.track" => Needs::Picture,
        "sequence.applyAudioTransition" => Needs::Sound,
        "graphics.set" | "graphics.newShape" | "graphics.setTextType" | "graphics.setCharStyle" | "graphics.align" => Needs::Graphic,
        "graphics.distribute" | "graphics.arrangeLayer" | "graphics.template.set" => Needs::Graphic,
        "effects.apply" | "effects.setParam" | "effects.addKeyframe" | "effects.setKeyframe" | "effects.setInterpolation" => effect_needs(params),
        "effects.deleteKeyframe" => effect_needs(params),
        _ => Needs::Clip,
    }
}

/// What an effect's act needs of its clips, as the engine's catalogue makes the effect `params` name by its id (the
/// only name the table admits there, see the module documentation): a video effect a clip of a video track, an audio
/// effect one of an audio track.
fn effect_needs(params: &Value) -> Needs {
    match params.get("effect").and_then(Value::as_str).and_then(find_effect).map(|effect| effect.kind) {
        Some(EffectKind::Video) => Needs::Picture,
        Some(EffectKind::Audio) => Needs::Sound,
        _ => Needs::Clip,
    }
}

/// A target as a call names it, for a refusal's detail: a name as it is, an id as its number.
fn shown(named: &Value) -> String {
    match named {
        Value::String(name) => name.clone(),
        other => other.to_string(),
    }
}

/// The video or audio track `name` names as the engine reads it (`track_p`): `V` or `A`, in either case, then its
/// number from 1.
fn by_name<'a>(sequence: &'a Sequence, name: &str) -> Option<&'a Track> {
    let mut rest = name.chars();
    let tracks = match rest.next()? {
        'v' | 'V' => &sequence.video_tracks,
        'a' | 'A' => &sequence.audio_tracks,
        _ => return None,
    };
    let number: usize = rest.as_str().parse().ok()?;
    tracks.get(number.checked_sub(1)?)
}

/// A call, and the active sequence its targets are checked in.
struct Call<'a> {
    session: &'a Session,
    sequence: &'a Sequence,
    command: &'a str,
    params: &'a Value,
}

impl Call<'_> {
    /// The act a refusal names: the command, and the effect it names.
    fn act(&self) -> String {
        match self.params.get("effect").and_then(Value::as_str) {
            Some(effect) => format!("`{}` of {effect}", self.command),
            None => format!("`{}`", self.command),
        }
    }

    /// Every clip the call names (`clip`, `clips`, each move's `clip`) is one of the sequence's, of the kind its act needs.
    fn clips(&self) -> Result<(), Miss> {
        let needs = needs(self.command, self.params);
        if let Some(id) = self.params.get("clip").and_then(Value::as_u64) {
            self.clip(id, needs, "/clip".to_string())?;
        }
        for (at, named) in self.params.get("clips").and_then(Value::as_array).into_iter().flatten().enumerate() {
            if let Some(id) = named.as_u64() {
                self.clip(id, needs, format!("/clips/{at}"))?;
            }
        }
        for (at, step) in self.params.get("moves").and_then(Value::as_array).into_iter().flatten().enumerate() {
            if let Some(id) = step.get("clip").and_then(Value::as_u64) {
                self.clip(id, needs, format!("/moves/{at}/clip"))?;
            }
        }
        Ok(())
    }

    /// Clip `id`, which the call names at `param`, is one of the sequence's, of the kind `needs`.
    fn clip(&self, id: u64, needs: Needs, param: String) -> Result<(), Miss> {
        let located = self.sequence.find_item(ClipId(id)).map(|(track, _)| track);
        let Some(track) = located else { return Err(Miss::new(NOT_FOUND, param, format!("clip {id} is not in the active sequence"))) };
        let video = self.sequence.track(track).is_some_and(|found| found.kind == TrackKind::Video);
        let (code, why) = match needs {
            Needs::Picture if !video => (OTHER_KIND, "is on an audio track"),
            Needs::Sound if video => (OTHER_KIND, "is on a video track"),
            Needs::Graphic if !filmcraft_engine::graphics::is_graphic(self.session, self.sequence, ClipId(id)) => (NOT_FOUND, "is no graphic clip"),
            _ => return Ok(()),
        };
        Err(Miss::new(code, param, format!("clip {id} {why}: {} {}", self.act(), needs.wants())))
    }

    /// Every track the call names is one of the sequence's (see the module documentation).
    fn tracks(&self) -> Result<(), Miss> {
        match self.command {
            "timeline.place" | "timeline.razor" | "sequence.closeGap" | "timeline.setTrack" => {
                for key in ["track", "audioTrack"] {
                    if let Some(named) = self.params.get(key) {
                        self.track(named, format!("/{key}"))?;
                    }
                }
                Ok(())
            }
            "timeline.move" => self.moves(),
            "captions.add" | "captions.setStyle" => self.caption_track(),
            "graphics.newText" => self.title_track(),
            _ => Ok(()),
        }
    }

    /// The kind of the video or audio track `named` names (at `param`), as the engine finds it (`track_p`): by its id,
    /// or as `V1` and `A1` on.
    fn track(&self, named: &Value, param: String) -> Result<TrackKind, Miss> {
        let found = match named.as_u64() {
            Some(id) => self.sequence.track(TrackId(id)),
            None => named.as_str().and_then(|name| by_name(self.sequence, name)),
        };
        found.map(|track| track.kind).ok_or_else(|| Miss::new(NO_TRACK, param, format!("track {} is not in the active sequence", shown(named))))
    }

    /// Each move's track is one of the sequence's, of its clip's kind: the engine would put a clip on a track of the other
    /// kind.
    fn moves(&self) -> Result<(), Miss> {
        for (at, step) in self.params.get("moves").and_then(Value::as_array).into_iter().flatten().enumerate() {
            let Some(named) = step.get("track") else { continue };
            let to = self.track(named, format!("/moves/{at}/track"))?;
            let clip = step.get("clip").and_then(Value::as_u64).and_then(|id| self.sequence.find_item(ClipId(id)));
            let from = clip.and_then(|(track, _)| self.sequence.track(track)).map(|track| track.kind);
            if from.is_some_and(|from| from != to) {
                let id = step.get("clip").map(shown).unwrap_or_default();
                let why = format!("clip {id} cannot move to track {}: a clip moves to a track of its own kind", shown(named));
                return Err(Miss::new(OTHER_KIND, format!("/moves/{at}/track"), why));
            }
        }
        Ok(())
    }

    /// The caption track the call names (`track`) is one of the sequence's, as the engine finds it: by its id, or as `C1`
    /// and on. The first caption of a sequence without a caption track may name `C1`: the engine makes that track for it.
    fn caption_track(&self) -> Result<(), Miss> {
        let Some(named) = self.params.get("track") else { return Ok(()) };
        let tracks = &self.sequence.caption_tracks;
        let made = self.command == "captions.add" && tracks.is_empty();
        let found = match named {
            Value::Number(id) => id.as_u64().is_some_and(|id| tracks.iter().any(|track| track.id.0 == id)),
            Value::String(name) => {
                let index = name.trim_start_matches(['C', 'c']).parse::<usize>().ok().and_then(|number| number.checked_sub(1));
                index.is_some_and(|at| at < tracks.len() || (made && at == 0))
            }
            _ => true,
        };
        if found {
            return Ok(());
        }
        Err(Miss::new(NOT_FOUND, "/track", format!("caption track {} is not in the active sequence", shown(named))))
    }

    /// A title's track (`track`, its index among the video tracks from V1: the engine reads no name there) is one of the
    /// sequence's: past the last one, the engine would take a free track for the title instead.
    fn title_track(&self) -> Result<(), Miss> {
        let Some(index) = self.params.get("track").and_then(Value::as_u64) else { return Ok(()) };
        if usize::try_from(index).is_ok_and(|at| at < self.sequence.video_tracks.len()) {
            return Ok(());
        }
        Err(Miss::new(NO_TRACK, "/track", format!("track V{} (index {index}) is not in the active sequence", index.saturating_add(1))))
    }

    /// Every caption the call names (`caption`, `captions`) is one of the sequence's.
    fn captions(&self) -> Result<(), Miss> {
        if let Some(id) = self.params.get("caption").and_then(Value::as_u64) {
            self.caption(id, "/caption".to_string())?;
        }
        for (at, named) in self.params.get("captions").and_then(Value::as_array).into_iter().flatten().enumerate() {
            if let Some(id) = named.as_u64() {
                self.caption(id, format!("/captions/{at}"))?;
            }
        }
        Ok(())
    }

    /// Caption `id`, which the call names at `param`, is one of the sequence's.
    fn caption(&self, id: u64, param: String) -> Result<(), Miss> {
        if self.sequence.find_caption(ClipId(id)).is_some() {
            return Ok(());
        }
        Err(Miss::new(NOT_FOUND, param, format!("caption {id} is not in the active sequence")))
    }

    /// The marker the call names (`marker`) is one of the active sequence's own: one session cuts several shorts, one
    /// sequence each, and another sequence's marker is not there.
    fn marker(&self) -> Result<(), Miss> {
        let Some(id) = self.params.get("marker").and_then(Value::as_u64) else { return Ok(()) };
        if self.sequence.markers.iter().any(|marker| marker.id.0 == id) {
            return Ok(());
        }
        Err(Miss::new(NOT_FOUND, "/marker", format!("marker {id} is not a marker of the active sequence")))
    }

    /// The transition the call names (`transition`) is on one of the sequence's tracks.
    fn transition(&self) -> Result<(), Miss> {
        let Some(id) = self.params.get("transition").and_then(Value::as_u64) else { return Ok(()) };
        if self.sequence.all_tracks().any(|track| track.transitions.iter().any(|transition| transition.id.0 == id)) {
            return Ok(());
        }
        Err(Miss::new(NOT_FOUND, "/transition", format!("transition {id} is not in the active sequence")))
    }
}
