//! `window.filmcraft`: the control channel / agent API for the page.
//!
//! The same JSON methods as the desktop control channel (`docs/control-protocol.md`) and the
//! same engine commands as MCP, as promises:
//!
//! ```js
//! await filmcraft.execute("sequence.razor", {seconds: 2})   // engine command → its result
//! await filmcraft.request("ui.inspect")                    // control method → its result
//! await filmcraft.commands()                               // command list
//! const {pngBase64} = await filmcraft.screenshot()         // the canvas as PNG (base64)
//! await filmcraft.importUrl("/clip.mp4")                   // fetch + import (tests)
//! await filmcraft.importFiles(input.files)                 // File / FileList / File[]
//! await filmcraft.openProject(file)                        // .fcproj File or URL
//! filmcraft.files(); await filmcraft.readFile("/exports/a.mp4"); filmcraft.info()
//! ```
//!
//! Results resolve with the method's `result`; errors reject with the error message.
//!
//! `filmcraft.executeChecked(revision, command, params, {activeSequence, deadlineMs})` runs one engine
//! command after checking the caller's revision and active sequence in the same UI-thread closure.
//! Since LTS slice R1 it runs only what its typed tables hold (`agent_view_policy::checked`): a
//! certified read with the keys it was certified with, or one of the legacy edits the LTS lab's
//! mirror adapter sends (`agent_view_policy::CHECKED_EDITS`, a place only as an overwrite), under the
//! keys and values of the `executeTyped` table. Every rejection it decides before anything runs is an
//! `Error` whose `code` is a registered `not_applied` code: `command_not_allowed` for a call outside
//! those tables (parameters that are not a JSON object included), `deadline_elapsed`,
//! `revision_conflict` or `sequence_changed` for a precondition that does not hold or comes in a form
//! it does not read, and `command_disabled` when the engine refuses the command itself before running
//! it. A rejection without a `code` failed after dispatch.
//!
//! In the LTS presentation (`?embedded`), `filmcraft.request` and `filmcraft.execute` send reads only
//! (`agent_view_policy::raw_request`, slice R1): the control channel's reads (the command list and the
//! menu tree, the interface state and its elements, the performance counters, a screenshot and the
//! timeline's hit tests) and, as a command, a certified read with the keys it was certified with.
//! Anything else rejects with an `Error` whose `code` is `command_not_allowed`, and nothing is sent:
//! an edit goes through `executeChecked` or `filmcraft.agentView.executeTyped`. The standalone
//! presentation sends every method.

use std::cell::RefCell;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

use filmcraft_engine::EngineError;
use filmcraft_ui_egui::ControlRequest;
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, future_to_promise};

use crate::agent_view_policy as policy;

type Waiting = (Receiver<Value>, js_sys::Function, js_sys::Function);

thread_local! {
    static TX: RefCell<Option<Sender<ControlRequest>>> = const { RefCell::new(None) };
    static WAITING: RefCell<Vec<Waiting>> = const { RefCell::new(Vec::new()) };
    static IMPORTED_FONTS: RefCell<Vec<(Vec<u8>, Vec<filmcraft_text::FaceId>)>> = const { RefCell::new(Vec::new()) };
}

pub fn set_sender(tx: Sender<ControlRequest>) {
    TX.with(|t| *t.borrow_mut() = Some(tx));
}

pub fn to_js(v: &Value) -> JsValue {
    js_sys::JSON::parse(&v.to_string()).unwrap_or(JsValue::NULL)
}

pub fn from_js(v: &JsValue) -> Value {
    if v.is_undefined() || v.is_null() {
        return Value::Null;
    }
    js_sys::JSON::stringify(v).ok().and_then(|s| s.as_string()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null)
}

/// `v` as JSON ([`from_js`]) when it is a JSON object, else null: what `executeChecked` reads its parameters and its
/// context from (null parameters are outside its tables, and a null context names no precondition).
fn object_from_js(v: &JsValue) -> Value {
    let value = from_js(v);
    if value.is_object() { value } else { Value::Null }
}

/// Resolve the promises of answered control requests (called every frame).
pub fn poll_replies() {
    WAITING.with(|w| {
        w.borrow_mut().retain(|(rx, resolve, reject)| match rx.try_recv() {
            Ok(v) => {
                if v.get("ok").and_then(Value::as_bool) == Some(true) {
                    let _ = resolve.call1(&JsValue::NULL, &to_js(v.get("result").unwrap_or(&Value::Null)));
                } else {
                    let msg = v.get("error").and_then(Value::as_str).unwrap_or("error").to_string();
                    let _ = reject.call1(&JsValue::NULL, &js_sys::Error::new(&msg));
                }
                false
            }
            Err(TryRecvError::Empty) => true,
            Err(TryRecvError::Disconnected) => {
                let _ = reject.call1(&JsValue::NULL, &js_sys::Error::new("request dropped"));
                false
            }
        });
    });
}

/// Send a control request; the promise settles when the UI thread answers it.
pub fn request(method: &str, params: Value) -> js_sys::Promise {
    let method = method.to_string();
    let p = js_sys::Promise::new(&mut |resolve, reject| {
        let (req, rx) = ControlRequest::new(method.clone(), params.clone());
        let sent = TX.with(|t| t.borrow().as_ref().map(|tx| tx.send(req).is_ok()).unwrap_or(false));
        if sent {
            WAITING.with(|w| w.borrow_mut().push((rx, resolve, reject)));
        } else {
            let _ = reject.call1(&JsValue::NULL, &js_sys::Error::new("The video editor is not running yet"));
        }
    });
    crate::repaint();
    p
}

fn set_fn(obj: &js_sys::Object, name: &str, f: JsValue) -> Result<(), JsValue> {
    js_sys::Reflect::set(obj, &name.into(), &f).map(|_| ())
}

fn files_of(v: &JsValue) -> Vec<web_sys::File> {
    if let Some(f) = v.dyn_ref::<web_sys::File>() {
        return vec![f.clone()];
    }
    if let Some(list) = v.dyn_ref::<web_sys::FileList>() {
        return (0..list.length()).filter_map(|i| list.get(i)).collect();
    }
    if js_sys::Array::is_array(v) {
        return js_sys::Array::from(v).iter().filter_map(|x| x.dyn_into::<web_sys::File>().ok()).collect();
    }
    Vec::new()
}

/// Fetch a URL into a `File` named after it (or `name`).
pub async fn fetch_file(url: &str, name: Option<String>) -> Result<web_sys::File, JsValue> {
    let w = web_sys::window().ok_or("no window")?;
    let resp: web_sys::Response = JsFuture::from(w.fetch_with_str(url)).await?.dyn_into()?;
    if !resp.ok() {
        return Err(format!("{url}: HTTP {}", resp.status()).into());
    }
    let blob: web_sys::Blob = JsFuture::from(resp.blob()?).await?.dyn_into()?;
    let name = name.unwrap_or_else(|| url.split(['?', '#']).next().unwrap_or(url).rsplit('/').next().unwrap_or("media").to_string());
    let parts = js_sys::Array::of1(&blob);
    web_sys::File::new_with_blob_sequence(&parts, &name)
}

/// Install `window.filmcraft`.
pub fn install() -> Result<(), JsValue> {
    let obj = js_sys::Object::new();
    set_fn(
        &obj,
        "projectSnapshot",
        Closure::<dyn Fn() -> js_sys::Promise>::new(|| {
            future_to_promise(caught(async move {
                let bytes = crate::import::on_ui(|w, _| {
                    let session = &w.app.session;
                    filmcraft_format::encode_with_view(&session.project, Some(&session.project_view()), false)
                })
                .await;
                if bytes.len() > 8 * 1024 * 1024 {
                    return Err(JsValue::from_str("project snapshot exceeds 8 MiB"));
                }
                Ok(js_sys::Uint8Array::from(bytes.as_slice()).into())
            }))
        })
        .into_js_value(),
    )?;
    set_fn(
        &obj,
        "importFontBytes",
        Closure::<dyn Fn(js_sys::Uint8Array, String) -> js_sys::Promise>::new(|bytes: js_sys::Uint8Array, text: String| {
            future_to_promise(caught(async move {
                if bytes.length() == 0 || bytes.length() > 2 * 1024 * 1024 || text.chars().count() > 4096 {
                    return Err(JsValue::from_str("font bytes or coverage text exceed bounds"));
                }
                let data = bytes.to_vec();
                let result = crate::import::on_ui(move |_, ctx| {
                    let (faces, reused) = IMPORTED_FONTS.with(|registered| {
                        let mut registered = registered.borrow_mut();
                        if let Some((_, faces)) = registered.iter().find(|(bytes, _)| *bytes == data) {
                            return Ok((faces.clone(), true));
                        }
                        let count = filmcraft_text::sfnt::read_faces_bytes(&data).len();
                        let total_bytes: usize = registered.iter().map(|(bytes, _)| bytes.len()).sum();
                        let total_faces: usize = registered.iter().map(|(_, faces)| faces.len()).sum();
                        if count == 0 || total_bytes + data.len() > 8 * 1024 * 1024 || total_faces + count > 32 {
                            return Err("font is invalid or the session font budget is exhausted".to_string());
                        }
                        let faces = filmcraft_text::fonts::add_font_data(data.clone());
                        registered.push((data, faces.clone()));
                        filmcraft_text::layout::clear_cache();
                        Ok((faces, false))
                    })?;
                    let values: Vec<Value> = faces
                        .into_iter()
                        .map(|id| {
                            let face = filmcraft_text::fonts::face(id);
                            let missing: Vec<String> = text
                                .chars()
                                .filter(|c| {
                                    !c.is_control()
                                        && !matches!(*c, '\u{200c}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{061c}' | '\u{feff}')
                                        && !face.has_char(*c)
                                })
                                .map(|c| format!("U+{:04X}", c as u32))
                                .collect();
                            let active = filmcraft_text::fonts::resolve(&face.info.family, &face.info.style).face == id;
                            json!({"family": face.info.family, "style": face.info.style, "active": active, "missingCodepoints": missing})
                        })
                        .collect();
                    ctx.request_repaint();
                    Ok::<_, String>(json!({"faces": values, "reused": reused}))
                })
                .await;
                result.map(|value| to_js(&value)).map_err(|error| JsValue::from_str(&error))
            }))
        })
        .into_js_value(),
    )?;
    set_fn(
        &obj,
        "textCoverage",
        Closure::<dyn Fn(String, String, f32) -> js_sys::Promise>::new(|text: String, family: String, size: f32| {
            future_to_promise(caught(async move {
                if text.chars().count() > 4096 || family.len() > 800 || !size.is_finite() || !(1.0..=1024.0).contains(&size) {
                    return Err(JsValue::from_str("text coverage parameters exceed bounds"));
                }
                let result = crate::import::on_ui(move |_, _| {
                    let style = filmcraft_text::TextStyle { family, size, ..Default::default() };
                    let layout = filmcraft_text::layout::layout_uncached(&text, &style, &Default::default());
                    let tofu = layout.glyphs.iter().filter(|glyph| glyph.id == 0).count();
                    json!({"missingFont": layout.missing_font, "tofuGlyphs": tofu, "glyphs": layout.glyphs.len()})
                })
                .await;
                Ok(to_js(&result))
            }))
        })
        .into_js_value(),
    )?;
    set_fn(
        &obj,
        "revision",
        Closure::<dyn Fn() -> js_sys::Promise>::new(|| {
            future_to_promise(caught(async move {
                let revision = crate::import::on_ui(|w, _| w.app.session.revision.to_string()).await;
                Ok(JsValue::from_str(&revision))
            }))
        })
        .into_js_value(),
    )?;
    set_fn(
        &obj,
        "context",
        Closure::<dyn Fn() -> js_sys::Promise>::new(|| {
            future_to_promise(caught(async move {
                let value = crate::import::on_ui(|w, _| {
                    json!({
                        "revision": w.app.session.revision.to_string(),
                        "activeSequence": w.app.session.state.active_sequence.map(|id| id.0.to_string()),
                    })
                })
                .await;
                Ok(to_js(&value))
            }))
        })
        .into_js_value(),
    )?;
    set_fn(
        &obj,
        "executeChecked",
        Closure::<dyn Fn(String, String, JsValue, JsValue) -> js_sys::Promise>::new(|revision: String, command: String, params: JsValue, context: JsValue| {
            let (params, context) = (object_from_js(&params), object_from_js(&context));
            future_to_promise(caught(async move {
                // its typed tables first, then the form of its preconditions (slice R1): a refusal carries its code, and
                // nothing has run
                let outside = |code: &str| rejection(code, &format!("{code}: executeChecked does not run `{command}` with these parameters"));
                policy::checked(&command, &params, policy::Runtime::HERE).map_err(outside)?;
                let expect = policy::CheckedExpect::from_call(&revision, &context).map_err(|(code, message)| rejection(code, &message))?;
                let result = crate::import::on_ui(move |w, _| {
                    // the preconditions just before the dispatch, and the engine's own refusal, each with its code
                    let session = &mut w.app.session;
                    let active_sequence = session.state.active_sequence.map(|id| id.0);
                    let guard = policy::checked_guard(&expect, js_sys::Date::now(), session.revision, active_sequence);
                    guard.map_err(|(code, message)| (Some(code), message))?;
                    let before = session.revision;
                    match session.execute(&command, params) {
                        Ok(result) => Ok(json!({"result": result, "revision": session.revision.to_string()})),
                        Err(error) => {
                            let disabled = match &error {
                                EngineError::Disabled(named, _) => Some(named.as_str()),
                                _ => None,
                            };
                            Err((policy::checked_failure(&command, disabled, before, session.revision), error.to_string()))
                        }
                    }
                })
                .await;
                match result {
                    Ok(value) => Ok(to_js(&value)),
                    Err((Some(code), message)) => Err(rejection(code, &message)),
                    Err((None, message)) => Err(JsValue::from_str(&message)),
                }
            }))
        })
        .into_js_value(),
    )?;
    set_fn(
        &obj,
        "request",
        Closure::<dyn Fn(String, JsValue) -> js_sys::Promise>::new(|method: String, params: JsValue| {
            let p = from_js(&params);
            by_name(&method, if p.is_null() { json!({}) } else { p })
        })
        .into_js_value(),
    )?;
    set_fn(
        &obj,
        "execute",
        Closure::<dyn Fn(String, JsValue) -> js_sys::Promise>::new(|command: String, params: JsValue| {
            let p = from_js(&params);
            by_name("engine.execute", json!({"command": command, "params": if p.is_null() { json!({}) } else { p }}))
        })
        .into_js_value(),
    )?;
    set_fn(&obj, "commands", Closure::<dyn Fn() -> js_sys::Promise>::new(|| request("engine.commands", json!({}))).into_js_value())?;
    set_fn(&obj, "inspect", Closure::<dyn Fn() -> js_sys::Promise>::new(|| request("ui.inspect", json!({}))).into_js_value())?;
    set_fn(
        &obj,
        "screenshot",
        Closure::<dyn Fn(JsValue) -> js_sys::Promise>::new(|params: JsValue| {
            let p = from_js(&params);
            request("ui.screenshot", if p.is_null() { json!({}) } else { p })
        })
        .into_js_value(),
    )?;
    set_fn(
        &obj,
        "info",
        Closure::<dyn Fn() -> JsValue>::new(|| {
            let mut v = crate::info();
            v["webcodecsStats"] = crate::webcodecs::stats();
            v["fetchesInFlight"] = json!(crate::fs::inflight());
            to_js(&v)
        })
        .into_js_value(),
    )?;
    set_fn(
        &obj,
        "files",
        Closure::<dyn Fn() -> JsValue>::new(|| {
            to_js(&Value::Array(crate::fs::list().into_iter().map(|(p, n, k)| json!({"path": p, "size": n, "kind": k})).collect()))
        })
        .into_js_value(),
    )?;
    set_fn(
        &obj,
        "readFile",
        Closure::<dyn Fn(String) -> js_sys::Promise>::new(|path: String| {
            future_to_promise(caught(async move {
                let blob = crate::fs::blob(&path).ok_or_else(|| JsValue::from_str(&format!("{path}: not found")))?;
                let buf = JsFuture::from(blob.array_buffer()).await?;
                Ok(js_sys::Uint8Array::new(&buf).into())
            }))
        })
        .into_js_value(),
    )?;
    set_fn(
        &obj,
        "importFiles",
        Closure::<dyn Fn(JsValue) -> js_sys::Promise>::new(|files: JsValue| {
            let files = files_of(&files);
            future_to_promise(async move { Ok(to_js(&crate::import::import_files(files).await)) })
        })
        .into_js_value(),
    )?;
    set_fn(
        &obj,
        "importUrl",
        Closure::<dyn Fn(String, JsValue) -> js_sys::Promise>::new(|url: String, name: JsValue| {
            future_to_promise(caught(async move {
                let f = fetch_file(&url, name.as_string()).await?;
                Ok(to_js(&crate::import::import_files(vec![f]).await))
            }))
        })
        .into_js_value(),
    )?;
    set_fn(
        &obj,
        "openProject",
        Closure::<dyn Fn(JsValue) -> js_sys::Promise>::new(|src: JsValue| {
            future_to_promise(caught(async move {
                let file = match src.as_string() {
                    Some(url) => fetch_file(&url, None).await?,
                    None => files_of(&src).into_iter().next().ok_or("need a File or URL")?,
                };
                crate::import::open_project_file(file).await.map(|v| to_js(&v)).map_err(|e| JsValue::from(js_sys::Error::new(&e)))
            }))
        })
        .into_js_value(),
    )?;
    let w = web_sys::window().ok_or("no window")?;
    js_sys::Reflect::set(&w, &"filmcraft".into(), &obj)?;
    Ok(())
}

/// [`request`] for the members a page script calls by name, `filmcraft.request` and `filmcraft.execute` (slice R1): in
/// the LTS presentation (`?embedded`) they send only what `agent_view_policy::raw_request` admits, reads, and anything
/// else rejects with an `Error` whose `code` is `command_not_allowed`, with nothing sent. The standalone presentation
/// sends every method, as upstream does.
fn by_name(method: &str, params: Value) -> js_sys::Promise {
    let refused = if crate::embedded() { policy::raw_request(method, &params).err() } else { None };
    if let Some(code) = refused {
        let named = |key: &str| params.get(key).and_then(Value::as_str);
        let command = if matches!(method, "engine.execute" | "ui.menu.invoke") { named("command").or_else(|| named("id")) } else { None };
        let message = format!("{code}: the LTS presentation sends only reads by name, not `{}`", command.unwrap_or(method));
        return js_sys::Promise::reject(&rejection(code, &message));
    }
    request(method, params)
}

/// A rejection decided before anything runs (slice R1): an `Error` with `message` whose `code` is the refusal's
/// registered `not_applied` code, as the read failures of `filmcraft.agentView` carry theirs.
fn rejection(code: &str, message: &str) -> JsValue {
    let error = js_sys::Error::new(message);
    let _ = js_sys::Reflect::set(&error, &JsValue::from_str("code"), &JsValue::from_str(code));
    error.into()
}

/// `f`, rejecting with an [`as_error`] `Error`.
async fn caught(f: impl std::future::Future<Output = Result<JsValue, JsValue>>) -> Result<JsValue, JsValue> {
    f.await.map_err(as_error)
}

/// Every member rejects with an `Error` (#90), as `execute` and `request` do, so `e.message` and
/// `e instanceof Error` work: a plain string (a fetch status, "not found") becomes one, and an
/// `Error` passes through unchanged.
fn as_error(e: JsValue) -> JsValue {
    if e.is_instance_of::<js_sys::Error>() {
        return e;
    }
    let msg = e.as_string().unwrap_or_else(|| format!("{e:?}"));
    js_sys::Error::new(&msg).into()
}
