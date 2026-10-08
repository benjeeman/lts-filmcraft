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

use std::cell::RefCell;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

use filmcraft_ui_egui::ControlRequest;
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, future_to_promise};

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

fn checked_from_js(v: &JsValue) -> Result<Value, JsValue> {
    let text = js_sys::JSON::stringify(v)?.as_string().ok_or_else(|| JsValue::from_str("JSON object required"))?;
    let value: Value = serde_json::from_str(&text).map_err(|_| JsValue::from_str("invalid JSON or Unicode"))?;
    if !value.is_object() {
        return Err(JsValue::from_str("JSON object required"));
    }
    Ok(value)
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
            let _ = reject.call1(&JsValue::NULL, &js_sys::Error::new("FilmCraft is not running yet"));
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
            let params = checked_from_js(&params);
            let context = checked_from_js(&context);
            future_to_promise(caught(async move {
                let params = params?;
                let context = context?;
                let expected = revision.parse::<u64>().map_err(|_| JsValue::from_str("invalid decimal project revision"))?;
                let expected_sequence = match context.get("activeSequence") {
                    Some(Value::Null) => None,
                    Some(Value::String(id)) => Some(id.parse::<u64>().map_err(|_| JsValue::from_str("invalid decimal sequence id"))?),
                    _ => return Err(JsValue::from_str("explicit activeSequence string or null is required")),
                };
                let deadline = context
                    .get("deadlineMs")
                    .and_then(Value::as_f64)
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| JsValue::from_str("finite deadlineMs is required"))?;
                let result = crate::import::on_ui(move |w, _| {
                    let session = &mut w.app.session;
                    if js_sys::Date::now() > deadline {
                        return Err("command deadline elapsed before dispatch".to_string());
                    }
                    if session.revision != expected {
                        return Err(format!("project revision conflict: expected {expected}, current {}", session.revision));
                    }
                    if session.state.active_sequence.map(|id| id.0) != expected_sequence {
                        return Err("active sequence changed before dispatch".to_string());
                    }
                    session
                        .execute(&command, params)
                        .map(|result| json!({"result": result, "revision": session.revision.to_string()}))
                        .map_err(|e| e.to_string())
                })
                .await;
                result.map(|v| to_js(&v)).map_err(|e| JsValue::from_str(&e))
            }))
        })
        .into_js_value(),
    )?;
    set_fn(
        &obj,
        "request",
        Closure::<dyn Fn(String, JsValue) -> js_sys::Promise>::new(|method: String, params: JsValue| {
            let p = from_js(&params);
            request(&method, if p.is_null() { json!({}) } else { p })
        })
        .into_js_value(),
    )?;
    set_fn(
        &obj,
        "execute",
        Closure::<dyn Fn(String, JsValue) -> js_sys::Promise>::new(|command: String, params: JsValue| {
            let p = from_js(&params);
            request("engine.execute", json!({"command": command, "params": if p.is_null() { json!({}) } else { p }}))
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
