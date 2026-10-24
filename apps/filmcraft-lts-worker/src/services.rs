//! The worker's contained file services: the only way the engine reaches a file.
//!
//! The engine names files by path ([`filmcraft_engine::Services`]). In the worker those paths are
//! virtual and map into the session workspace only:
//!
//! - `/files/lts-<sha256>.<ext>`: a staged source or font, read-only, and only once the worker has
//!   staged it (its bytes hash to its name; never a reparse point);
//! - `/projects/<name>`: the project folder (`project/`), read and written;
//! - `/outputs/<requestId>/<name>`: an export's folder (`outputs/<requestId>/`), written only;
//! - `/lts-exports/<name>`: the folder the typed interchange exports write in
//!   ([`crate::agent_view_policy::EXPORT_ROOT`]), mapped to `outputs/lts-exports/`.
//!
//! Every other path is refused: an absolute or drive path, a `..` or `.` segment, a name that is
//! not staged, another root. Nothing is listed, and no volume or home folder is reported. The
//! agent never names a path: the host and the worker build every one (INV-12, INV-47). No folder
//! or file of the workspace may be a link or a junction: each level is checked before it is used,
//! so a reparse point planted in the workspace never leads outside it.

use std::collections::BTreeSet;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

use sha2::{Digest, Sha256};

/// The folder of staged files in the workspace.
pub const FILES: &str = "files";

/// The project folder in the workspace.
pub const PROJECT: &str = "project";

/// The folder of exports in the workspace.
pub const OUTPUTS: &str = "outputs";

/// The folder of model-visible images in the workspace.
pub const PREVIEWS: &str = "previews";

/// The folder under [`OUTPUTS`] that the typed interchange exports write in.
const INTERCHANGE: &str = "lts-exports";

/// The longest segment of a workspace path, in bytes.
const MAX_SEGMENT: usize = 160;

/// The longest file extension of a staged name.
const MAX_EXTENSION: usize = 8;

/// Why staging refused a file: a registered code and a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub code: &'static str,
    pub message: String,
}

impl Refusal {
    /// A refusal with the registered `code` and why.
    pub fn new(code: &'static str, message: impl Into<String>) -> Refusal {
        Refusal { code, message: message.into() }
    }
}

/// A file staged into the workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Staged {
    /// The engine's path of it (`/files/lts-<sha256>.<ext>`).
    pub path: String,
    pub bytes: u64,
}

/// A refusal of the services: nothing was read or written.
fn denied(path: &str, why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, format!("{path}: {why}"))
}

/// Whether `segment` is a plain name: ASCII letters, digits, `.`, `_` and `-`, neither starting
/// nor ending with `.`, at most [`MAX_SEGMENT`] bytes, and no Windows device (`CON`, `NUL.mp4`,
/// `com1`). So never `..`, `.`, a drive, a separator or an empty name.
pub fn plain(segment: &str) -> bool {
    let chars = segment.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    let ends = !segment.starts_with('.') && !segment.ends_with('.');
    chars && ends && !segment.is_empty() && segment.len() <= MAX_SEGMENT && !device(segment)
}

/// Whether `segment` names a Windows device, in any case, with or without an extension.
fn device(segment: &str) -> bool {
    let stem = segment.split('.').next().unwrap_or_default().to_ascii_uppercase();
    match stem.as_bytes() {
        b"CON" | b"PRN" | b"AUX" | b"NUL" => true,
        [b'C', b'O', b'M', digit] | [b'L', b'P', b'T', digit] => digit.is_ascii_digit(),
        _ => false,
    }
}

/// The lowercase hex sha256 and the extension of a staged name `lts-<64 hex>.<ext>`.
pub fn staged_name(name: &str) -> Option<(&str, &str)> {
    let (hex, ext) = name.strip_prefix("lts-")?.split_once('.')?;
    let hex_ok = hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    let ext_ok = (1..=MAX_EXTENSION).contains(&ext.len()) && ext.bytes().all(|b| b.is_ascii_digit() || b.is_ascii_lowercase());
    (hex_ok && ext_ok).then_some((hex, ext))
}

/// The lowercase hex sha256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The size and the lowercase hex sha256 of the file at `path`, read a MiB at a time.
pub fn file_digest(path: &Path) -> io::Result<(u64, String)> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut chunk = vec![0u8; 1 << 20];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        hasher.update(&chunk[..n]);
        size += n as u64;
    }
    Ok((size, hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()))
}

/// Whether the entry the metadata describes (read without following it) is a link or a junction:
/// the workspace never follows one.
fn linked(meta: &std::fs::Metadata) -> bool {
    meta.file_type().is_symlink()
}

/// A workspace path the engine named, and what the engine may do there.
enum Place {
    /// A staged file (its name in `files/`): read only.
    Staged(String),
    /// A file of the project folder (its name in `project/`): read and written.
    Project(String),
    /// A file of an export folder (its folders under the workspace, and its name): written only.
    Output(Vec<String>, String),
}

/// The contained [`filmcraft_engine::Services`] of one session workspace.
#[derive(Debug)]
pub struct ContainedServices {
    root: PathBuf,
    /// The names under `files/` staged so far (`lts-<sha256>.<ext>`).
    staged: RwLock<BTreeSet<String>>,
    /// How many files the services wrote so far.
    written: AtomicU64,
}

impl ContainedServices {
    /// The services of the session workspace `root`, an existing folder.
    pub fn new(root: &Path) -> io::Result<ContainedServices> {
        if !std::fs::metadata(root)?.is_dir() {
            return Err(io::Error::new(io::ErrorKind::NotADirectory, format!("{} is not a folder", root.display())));
        }
        Ok(ContainedServices { root: root.to_path_buf(), staged: RwLock::new(BTreeSet::new()), written: AtomicU64::new(0) })
    }

    /// The session workspace.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// How many files the services wrote so far, each whole (`filmcraft_format::atomic_write`: a write that fails
    /// leaves nothing and counts nothing). The witness of an edit counts them before and after the engine runs it: a
    /// file the engine wrote is part of what the edit changed.
    pub fn written(&self) -> u64 {
        self.written.load(Ordering::SeqCst)
    }

    /// The folder `parts` under the workspace, made one level at a time when `make`; every level
    /// that exists must be a plain folder, never a link or a junction.
    fn folder<S: AsRef<str>>(&self, parts: &[S], make: bool) -> io::Result<PathBuf> {
        let mut dir = self.root.clone();
        for part in parts {
            dir.push(part.as_ref());
            match std::fs::symlink_metadata(&dir) {
                Ok(meta) if meta.is_dir() && !linked(&meta) => {}
                Ok(_) => return Err(denied(&dir.display().to_string(), "not a plain folder of the workspace")),
                Err(e) if make && e.kind() == io::ErrorKind::NotFound => std::fs::create_dir(&dir)?,
                Err(e) => return Err(e),
            }
        }
        Ok(dir)
    }

    /// The file `name` in the folder `parts`: an existing plain file, or a name not taken yet when
    /// `absent_ok` (the folders are then made when `make`).
    fn file_in<S: AsRef<str>>(&self, parts: &[S], name: &str, make: bool, absent_ok: bool) -> io::Result<PathBuf> {
        let path = self.folder(parts, make)?.join(name);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() && !linked(&meta) => Ok(path),
            Ok(_) => Err(denied(&path.display().to_string(), "not a plain file of the workspace")),
            Err(e) if absent_ok && e.kind() == io::ErrorKind::NotFound => Ok(path),
            Err(e) => Err(e),
        }
    }

    /// Where the engine's `path` lies in the workspace, or the refusal (nothing is touched).
    fn place(&self, path: &str) -> io::Result<Place> {
        let segments: Vec<&str> = path.strip_prefix('/').map(|rest| rest.split('/').collect()).unwrap_or_default();
        match segments.as_slice() {
            [FILES, name] if self.is_staged(name) => Ok(Place::Staged(name.to_string())),
            ["projects", name] if plain(name) => Ok(Place::Project(name.to_string())),
            [OUTPUTS, request, name] if plain(request) && plain(name) => Ok(Place::Output(vec![OUTPUTS.to_string(), request.to_string()], name.to_string())),
            [INTERCHANGE, folders @ .., name] if plain(name) && folders.iter().all(|s| plain(s)) => {
                let folders = [OUTPUTS, INTERCHANGE].iter().chain(folders).map(|s| s.to_string()).collect();
                Ok(Place::Output(folders, name.to_string()))
            }
            _ => Err(denied(path, "outside the session workspace")),
        }
    }

    /// The workspace file the engine may read at `path`: a staged file or a project file.
    fn readable(&self, path: &str) -> io::Result<PathBuf> {
        match self.place(path)? {
            Place::Staged(name) => self.file_in(&[FILES], &name, false, false),
            Place::Project(name) => self.file_in(&[PROJECT], &name, false, false),
            Place::Output(..) => Err(denied(path, "exports are written, never read")),
        }
    }

    fn is_staged(&self, name: &str) -> bool {
        self.staged.read().unwrap_or_else(PoisonError::into_inner).contains(name)
    }

    /// Stage the file the host placed at `rel` (`files/lts-<sha256>.<ext>`) for the handle
    /// `sha256:<hex>` of the same hash, `bytes` long when given: refused `invalid_params` when a
    /// name or the handle is of another form or names another hash, and `staged_file_corrupt` when
    /// the file is missing, is not a plain file (a link or a junction), has another size or does not
    /// hash to its name. Staging a staged file again changes nothing.
    pub fn stage(&self, handle: &str, rel: &str, bytes: Option<u64>) -> Result<Staged, Refusal> {
        let named = rel.strip_prefix("files/").and_then(|name| staged_name(name).map(|(hex, _)| (name, hex)));
        let Some((name, hex)) = named else {
            return Err(Refusal::new("invalid_params", format!("{rel} is not a staged name (files/lts-<sha256>.<ext>)")));
        };
        if handle.strip_prefix("sha256:") != Some(hex) {
            return Err(Refusal::new("invalid_params", format!("the handle {handle} does not name the file {rel}")));
        }
        let corrupt = |why: String| Refusal::new("staged_file_corrupt", format!("{rel}: {why}"));
        let path = self.file_in(&[FILES], name, false, false).map_err(|e| corrupt(e.to_string()))?;
        let (size, sha) = file_digest(&path).map_err(|e| corrupt(e.to_string()))?;
        if let Some(claimed) = bytes.filter(|&claimed| claimed != size) {
            return Err(corrupt(format!("{size} bytes, not the {claimed} the host staged")));
        }
        if sha != hex {
            return Err(corrupt(format!("the bytes hash to {sha}, not to the name")));
        }
        self.staged.write().unwrap_or_else(PoisonError::into_inner).insert(name.to_string());
        Ok(Staged { path: format!("/{FILES}/{name}"), bytes: size })
    }

    /// The engine's path of the staged file of `handle` (`sha256:<hex>`), when it is staged.
    pub fn staged_path(&self, handle: &str) -> Option<String> {
        let hex = handle.strip_prefix("sha256:")?;
        let staged = self.staged.read().unwrap_or_else(PoisonError::into_inner);
        let name = staged.iter().find(|name| staged_name(name).is_some_and(|(sha, _)| sha == hex))?;
        Some(format!("/{FILES}/{name}"))
    }

    /// The bytes of the project file `name` (a plain name in `project/`), for the worker's own open.
    pub fn read_project(&self, name: &str) -> io::Result<Vec<u8>> {
        if !plain(name) {
            return Err(denied(name, "not a project file name"));
        }
        std::fs::read(self.file_in(&[PROJECT], name, false, false)?)
    }

    /// The folder of the request `request` under `outputs/`, made if missing.
    pub fn output_folder(&self, request: &str) -> io::Result<PathBuf> {
        if !plain(request) {
            return Err(denied(request, "not a request folder name"));
        }
        self.folder(&[OUTPUTS, request], true)
    }

    /// Write `data` as the plain file `name` of the workspace folder `parts` (made if missing), in
    /// place of any plain file of that name, whole or not at all ([`ContainedServices::written`]);
    /// answers where it is.
    pub fn write_in<S: AsRef<str>>(&self, parts: &[S], name: &str, data: &[u8]) -> io::Result<PathBuf> {
        if !plain(name) || !parts.iter().all(|part| plain(part.as_ref())) {
            return Err(denied(name, "not a workspace file name"));
        }
        let path = self.file_in(parts, name, true, true)?;
        filmcraft_format::atomic_write(&path, data)?;
        self.written.fetch_add(1, Ordering::SeqCst);
        Ok(path)
    }

    /// Write `data` as the plain file `name` of the workspace folder `parts` (made if missing) when
    /// no file of that name is there; a plain file of that name already there is kept as it is,
    /// never replaced (a checkpoint's name holds the hash of its bytes). Answers where it is.
    pub fn write_new<S: AsRef<str>>(&self, parts: &[S], name: &str, data: &[u8]) -> io::Result<PathBuf> {
        if !plain(name) || !parts.iter().all(|part| plain(part.as_ref())) {
            return Err(denied(name, "not a workspace file name"));
        }
        let path = self.file_in(parts, name, true, true)?;
        if !path.is_file() {
            filmcraft_format::atomic_write(&path, data)?;
            self.written.fetch_add(1, Ordering::SeqCst);
        }
        Ok(path)
    }
}

impl filmcraft_engine::Services for ContainedServices {
    fn read_file(&self, path: &str) -> io::Result<Vec<u8>> {
        std::fs::read(self.readable(path)?)
    }

    fn write_file(&self, path: &str, data: &[u8]) -> io::Result<()> {
        match self.place(path)? {
            Place::Staged(_) => Err(denied(path, "staged files are read-only")),
            Place::Project(name) => self.write_in(&[PROJECT], &name, data).map(|_| ()),
            Place::Output(folders, name) => self.write_in(&folders, &name, data).map(|_| ()),
        }
    }

    fn file_size(&self, path: &str) -> io::Result<u64> {
        Ok(std::fs::metadata(self.readable(path)?)?.len())
    }

    fn read_range(&self, path: &str, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        use std::io::{Seek, SeekFrom};
        let mut file = std::fs::File::open(self.readable(path)?)?;
        file.seek(SeekFrom::Start(offset))?;
        let mut out = Vec::with_capacity(len.min(1 << 24));
        file.take(len as u64).read_to_end(&mut out)?;
        Ok(out)
    }

    fn reader(&self, path: &str) -> Option<io::Result<filmcraft_media::SharedReader>> {
        let opened = self.readable(path).and_then(|file| filmcraft_media::reader::FileReader::open(&file));
        Some(opened.map(|reader| Arc::new(reader) as filmcraft_media::SharedReader))
    }
}
