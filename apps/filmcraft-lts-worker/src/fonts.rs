//! The fonts a worker renders with (native-worker-architecture.md §5.3; RD-14; OC-33 E1): the faces
//! the engine bundles and the faces `hello` registers by their bytes, never the system's.
//!
//! `hello {fonts: [{name, path: "files/lts-<sha256>.<ext>", sha256}]}` stages each font file (its
//! bytes must hash to its name) and registers its faces, all of them or none. `ready` lists them
//! with the digest of the whole font set (`fontsetDigest`, INV-35), which every frames read and
//! every export result repeats. An edit that names a family no loaded face holds is refused
//! `font_not_loaded` before it runs (a registered font's `name`, such as `noto-sans-hebrew`, stands
//! for its family); a render of a project that names one anyway (a project made elsewhere) is
//! labelled with the families it substitutes (`substituted`, RD-14).

use filmcraft_engine::Services;
use filmcraft_engine::project::Project;
use serde_json::{Value, json};

use crate::services::{self, ContainedServices, Refusal};

/// The extensions of a font file.
const FONT_EXTENSIONS: [&str; 4] = ["ttf", "otf", "ttc", "otc"];

/// The longest name a registered font goes by, in bytes.
const MAX_NAME: usize = 64;

/// A font file `hello` registered.
#[derive(Clone, Debug)]
struct Registered {
    /// The name a call may give for its family (`noto-sans-hebrew`).
    name: String,
    /// The family of its faces, and their styles.
    family: String,
    styles: Vec<String>,
    sha256: String,
}

/// The fonts of a session: the files `hello` registered, and the digest of every face the engine
/// holds.
#[derive(Clone, Debug, Default)]
pub(crate) struct Fontset {
    registered: Vec<Registered>,
    pub(crate) digest: String,
}

impl Fontset {
    /// The set of the bundled faces alone.
    pub(crate) fn bundled() -> Fontset {
        Fontset { registered: Vec::new(), digest: digest() }
    }

    /// `ready`'s list of the registered fonts: name, family, styles and sha256.
    pub(crate) fn listing(&self) -> Value {
        let fonts: Vec<Value> = self.registered.iter().map(|f| json!({"name": f.name, "family": f.family, "styles": f.styles, "sha256": f.sha256})).collect();
        json!(fonts)
    }

    /// The family a call's `font` names: a registered font's `name` stands for its family.
    fn family(&self, font: &str) -> String {
        self.registered.iter().find(|f| f.name == font).map_or_else(|| font.to_string(), |f| f.family.clone())
    }

    /// `params` of `command` with each font family it names ([`font_pointers`]) given as a family
    /// the set holds (a registered font's `name` becomes its family), or why not: no loaded face
    /// holds the family, so the render would substitute another (`font_not_loaded`).
    pub(crate) fn check(&self, command: &str, mut params: Value) -> Result<Value, String> {
        for pointer in font_pointers(command) {
            let Some(font) = params.pointer(pointer).and_then(Value::as_str).map(str::to_string) else { continue };
            let family = self.family(&font);
            if !loaded(&family) {
                return Err(format!("no loaded face holds the family `{font}`: register its font file at hello"));
            }
            if family != font
                && let Some(slot) = params.pointer_mut(pointer)
            {
                *slot = json!(family);
            }
        }
        Ok(params)
    }
}

/// The JSON pointers of the font families a call of `command` may name.
fn font_pointers(command: &str) -> &'static [&'static str] {
    match command {
        "graphics.newText" | "captions.setStyle" => &["/font"],
        "graphics.setCharStyle" => &["/style/font"],
        "graphics.set" => &["/props/font"],
        _ => &[],
    }
}

/// Stage and register the font files `fonts` names (`hello`'s list; none when left out), all or
/// none: every file is staged and read, and must hold a face, before any face is registered.
pub(crate) fn register(services: &ContainedServices, fonts: Option<&Value>) -> Result<Fontset, Refusal> {
    let entries = match fonts {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(entries)) => entries.clone(),
        Some(_) => return Err(Refusal::new("invalid_params", "`fonts` is a list of {name, path, sha256}")),
    };
    let mut files = Vec::new();
    for entry in &entries {
        let text = |key: &str| entry.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        let (name, path, sha256) = (text("name"), text("path"), text("sha256"));
        if name.trim().is_empty() || name.len() > MAX_NAME || files.iter().any(|(other, _, _)| *other == name) {
            return Err(Refusal::new("invalid_params", format!("a font's name is 1 to {MAX_NAME} bytes, once in the list: `{name}`")));
        }
        let extension = path.rsplit_once('.').map(|(_, extension)| extension).unwrap_or_default();
        if !FONT_EXTENSIONS.contains(&extension) {
            return Err(Refusal::new("invalid_params", format!("{path} is not a font file (ttf, otf, ttc, otc)")));
        }
        let staged = services.stage(&format!("sha256:{sha256}"), &path, None)?;
        let bytes = services.read_file(&staged.path).map_err(|e| Refusal::new("staged_file_corrupt", format!("{path}: {e}")))?;
        if filmcraft_text::sfnt::read_faces_bytes(&bytes).is_empty() {
            return Err(Refusal::new("invalid_params", format!("{path} holds no font face")));
        }
        files.push((name, sha256, bytes));
    }
    let mut registered = Vec::new();
    for (name, sha256, bytes) in files {
        let faces: Vec<_> = filmcraft_text::fonts::add_font_data(bytes).into_iter().map(filmcraft_text::fonts::face).collect();
        let family = faces.first().map(|face| face.info.family.clone()).unwrap_or_default();
        let styles = faces.iter().map(|face| face.info.style.clone()).collect();
        registered.push(Registered { name, family, styles, sha256 });
    }
    Ok(Fontset { registered, digest: digest() })
}

/// The digest of every face the engine holds (`sha256:<hex>`, INV-35): its family, style, index in
/// its file and the sha256 of the file, one line a face, sorted.
fn digest() -> String {
    let faces = filmcraft_text::fonts::all_faces();
    let line = |face: &std::sync::Arc<filmcraft_text::fonts::Face>| {
        let sha = face.data().map(|bytes| services::sha256_hex(&bytes)).unwrap_or_default();
        format!("{}\t{}\t{}\t{sha}", face.info.family, face.info.style, face.info.index)
    };
    let mut lines: Vec<String> = faces.iter().map(line).collect();
    lines.sort();
    format!("sha256:{}", services::sha256_hex(lines.join("\n").as_bytes()))
}

/// Whether a loaded face holds `family` (ignoring case, as the engine resolves a family).
pub(crate) fn loaded(family: &str) -> bool {
    filmcraft_text::fonts::families().iter().any(|(name, _)| name.eq_ignore_ascii_case(family))
}

/// The families the project names for its text (titles and their character styles, caption
/// styles) that no loaded face holds, sorted: a render of it substitutes the default face for them
/// (RD-14).
pub(crate) fn substituted(project: &Project) -> Vec<String> {
    let mut named = Vec::new();
    if let Ok(view) = serde_json::to_value(project) {
        families_in(&view, &mut named);
    }
    named.retain(|family| !family.trim().is_empty() && !loaded(family));
    named.sort();
    named.dedup();
    named
}

/// The families `value` names under a `font` key, at any depth: as a string, or as the text of a
/// parameter (`{"value": {"Text": family}}`).
fn families_in(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                if key == "font" {
                    match inner {
                        Value::String(family) => out.push(family.clone()),
                        other => out.extend(other.pointer("/value/Text").and_then(Value::as_str).map(str::to_string)),
                    }
                }
                families_in(inner, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                families_in(item, out);
            }
        }
        _ => {}
    }
}

/// Which characters of `text` a render in `family` would draw from no face (neither the family's
/// nor a fallback holds them), whether the family is loaded, and the faces a fallback sets the
/// other characters in (`fallbacks`: family, style, origin and those characters, in text order):
/// what a title in that family would show. In a worker only the bundled and registered faces can
/// serve as fallbacks (the system font folders are never read).
pub(crate) fn coverage(family: &str, text: &str) -> Value {
    use filmcraft_text::fonts::{face, fallback_for, resolve};
    let resolved = resolve(family, "Regular");
    let mut missing: Vec<String> = Vec::new();
    let mut fallbacks: Vec<(filmcraft_text::fonts::FaceId, String)> = Vec::new();
    for c in text.chars().filter(|c| !c.is_whitespace() && !c.is_control()) {
        let id = fallback_for(c, resolved.face);
        if !face(id).has_char(c) {
            missing.push(c.to_string());
        } else if id != resolved.face {
            match fallbacks.iter_mut().find(|(f, _)| *f == id) {
                Some((_, chars)) => chars.push(c),
                None => fallbacks.push((id, c.to_string())),
            }
        }
    }
    let mut served = Vec::new();
    for (id, chars) in fallbacks {
        let f = face(id);
        served.push(json!({"family": f.info.family, "style": f.info.style, "origin": f.info.origin, "text": chars}));
    }
    let used = face(resolved.face).info.family.clone();
    json!({"family": family, "loaded": !resolved.missing, "face": used, "missing": missing, "fallbacks": served})
}
