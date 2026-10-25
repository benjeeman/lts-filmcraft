//! The exact vertical frame (spec §6.1 and §6.6; plan 63 C-6): [`exact_frame`] renders a plan's output frame with
//! `filmcraft_render::render_vertical`, through the media pool's full-resolution provider (never a proxy, never a
//! render-preview frame, never draft decoding) and the session's 16:9 frame cache, keyed by [`frame_identity`];
//! [`exact_frame_rgba8`] is the same frame as straight RGBA8 over black, the bytes M3's EXACT preview shows and T-U20
//! compares. No `vertical.*` command calls it: the only pixel analysis of the command table is `vertical.measureCuts`
//! (T-N21).

use filmcraft_project::ItemId;
use filmcraft_render::{FrameCache16x9, FrameIdentity, Image, VerticalRenderInputs, VerticalRenderOptions, render_vertical};
use filmcraft_time::Tick;
use serde_json::{Value, json};

use crate::{EngineError, Result, Session};

/// The identity of the session's picture (plan 63 C-7): its project epoch, picture revision and media pool generation,
/// the parts of the 16:9 frame cache's key that the session knows.
pub fn frame_identity(s: &Session) -> FrameIdentity {
    FrameIdentity { epoch: s.epoch, picture_revision: s.picture_revision, media_generation: s.media.generation() }
}

/// The exact frame of plan `plan` of sequence `seq` at tick `t`, at the plan's output size, with the plan's subtitles when
/// `captions`. Refused `not_found {sequence}` or `not_found {plan}`, and `invalid_plan {violations}` for a plan that breaks
/// an invariant (spec §2.8).
pub fn exact_frame(s: &Session, seq: ItemId, plan: u32, t: Tick, captions: bool) -> Result<Image> {
    let Some(q) = s.project.sequence(seq) else {
        return Err(refused("not_found", format!("there is no sequence {}", seq.0), json!({"sequence": seq.0})));
    };
    let planned = q.vertical.as_deref().is_some_and(|set| set.plans.iter().any(|p| p.id == plan));
    if !planned {
        return Err(refused("not_found", format!("the sequence has no plan {plan}"), json!({"plan": plan})));
    }
    let violations = crate::vertical::plan_violations(s, seq, plan);
    if !violations.is_empty() {
        return Err(refused("invalid_plan", "the plan breaks an invariant; vertical.repair fixes it", json!({"violations": violations})));
    }
    let sources = s.media.full_res_provider(s.project.clone(), s.services.clone());
    let cache: &dyn FrameCache16x9 = &*s.vertical_cache;
    let inputs = VerticalRenderInputs { sources: &sources, captions, working_output: false, cache: Some(cache), identity: frame_identity(s) };
    let frame = render_vertical(&s.project, seq, plan, t, VerticalRenderOptions::default(), &inputs);
    frame.ok_or_else(|| EngineError::Other(format!("the vertical frame of plan {plan} at tick {} could not be rendered", t.0)))
}

/// [`exact_frame`] as straight RGBA8 over black: `(width, height, bytes)`.
pub fn exact_frame_rgba8(s: &Session, seq: ItemId, plan: u32, t: Tick, captions: bool) -> Result<(usize, usize, Vec<u8>)> {
    let img = exact_frame(s, seq, plan, t, captions)?;
    let bytes = img.over_black_rgba8();
    Ok((img.w, img.h, bytes))
}

/// A refusal of the exact frame; the command that asks for it (if any) names itself (`EngineError::Invalid`'s empty
/// `cmd` is filled in by `Session::execute`).
fn refused(code: &'static str, message: impl Into<String>, detail: Value) -> EngineError {
    EngineError::Invalid { cmd: String::new(), code, message: message.into(), detail }
}
