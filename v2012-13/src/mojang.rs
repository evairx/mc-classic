//! Mojang's current servers: the version list, each version's JSON, and verified downloads that
//! stay cached on disk. Both ways of running a version use it, and so does the Java runtime.

use std::{
    collections::HashSet,
    ffi::OsString,
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Mutex, OnceLock, PoisonError,
    },
    thread,
    time::Duration,
};

use serde_json::Value;
use sha1::{Digest, Sha1};

use crate::Result;

const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
/// Parallel downloads. A modern version has thousands of small asset files.
const THREADS: usize = 16;
const ATTEMPTS: u64 = 3;

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(60))
            .build()
    })
}

fn get(url: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    agent().get(url).call()?.into_reader().read_to_end(&mut bytes)?;
    Ok(bytes)
}

pub fn get_text(url: &str) -> Result<String> {
    Ok(String::from_utf8(get(url)?)?)
}

pub fn get_json(url: &str) -> Result<Value> {
    Ok(serde_json::from_slice(&get(url)?)?)
}

/// Mojang's list of every version, with the latest release and snapshot.
pub fn manifest() -> Result<Value> {
    get_json(MANIFEST_URL)
}

/// Downloads a JSON file and refuses it unless its SHA-1 matches.
pub fn get_verified_json(url: &str, sha1: &str) -> Result<Value> {
    let bytes = get(url)?;
    check_sha1(&bytes, sha1, url)?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// The version's JSON, cached as `versions/<id>/<id>.json` in the game folder. The cached copy
/// is used while Mojang lists the same SHA-1, and also when Mojang cannot be reached.
pub fn version(game_dir: &Path, id: &str) -> Result<Value> {
    let path = game_dir.join("versions").join(id).join(format!("{id}.json"));
    let manifest = match manifest() {
        Ok(manifest) => manifest,
        Err(e) if path.is_file() => {
            log!("Sin conexión con Mojang ({e}); uso la copia guardada de {id}.");
            return Ok(serde_json::from_slice(&fs::read(&path)?)?);
        }
        Err(e) => return Err(format!("No se pudo leer la lista de versiones de Mojang: {e}").into()),
    };
    let listed = manifest["versions"]
        .as_array()
        .and_then(|versions| versions.iter().find(|v| v["id"] == id))
        .ok_or_else(|| format!("Mojang no publica la versión {id}."))?;
    let url = listed["url"].as_str().ok_or("la lista de versiones no trae url")?;
    let sha1 = listed["sha1"].as_str().ok_or("la lista de versiones no trae sha1")?;
    if let Ok(cached) = fs::read(&path) {
        if sha1_hex(&cached).eq_ignore_ascii_case(sha1) {
            return Ok(serde_json::from_slice(&cached)?);
        }
    }
    log!("descargando {url}");
    let bytes = get(url)?;
    check_sha1(&bytes, sha1, url)?;
    save(&path, &bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// The version's game jar, `versions/<id>/<id>.jar`. A version built on another one (Fabric)
/// names that one in `jar`.
pub fn client(meta: &Value, game_dir: &Path) -> Result<Download> {
    let id = meta["jar"].as_str().or(meta["id"].as_str()).ok_or("la versión no trae id")?;
    Download::new(&meta["downloads"]["client"], game_dir.join("versions").join(id).join(format!("{id}.jar")))
}

/// Libraries this machine needs: Windows, 64 bits.
pub fn libraries(meta: &Value) -> Result<Vec<&Value>> {
    Ok(meta["libraries"].as_array().ok_or("la versión no lista librerías")?.iter().filter(|lib| allowed(&lib["rules"])).collect())
}

/// The library's jar, if it has one, under `libraries/`.
pub fn artifact(library: &Value, libraries_dir: &Path) -> Result<Option<Download>> {
    library_file(&library["downloads"]["artifact"], libraries_dir)
}

/// The library's jar of Windows natives, if it has one. Libraries since 1.19 put their natives
/// in separate libraries instead, which `artifact` covers.
pub fn natives(library: &Value, libraries_dir: &Path) -> Result<Option<Download>> {
    let Some(classifier) = library["natives"]["windows"].as_str() else { return Ok(None) };
    library_file(&library["downloads"]["classifiers"][classifier.replace("${arch}", "64")], libraries_dir)
}

fn library_file(entry: &Value, libraries_dir: &Path) -> Result<Option<Download>> {
    if entry.is_null() {
        return Ok(None);
    }
    let path = entry["path"].as_str().ok_or("librería sin path")?;
    Ok(Some(Download::new(entry, under(libraries_dir, path)?)?))
}

/// Mojang's rules for this machine (Windows 10 or 11, 64 bits, no optional features). No rules
/// means allowed; otherwise the last rule that applies decides.
pub fn allowed(rules: &Value) -> bool {
    allowed_with(rules, &[])
}

/// `allowed`, with these optional features on, such as `has_custom_resolution`.
pub fn allowed_with(rules: &Value, features: &[&str]) -> bool {
    let Some(rules) = rules.as_array() else { return true };
    let mut allowed = false;
    for rule in rules.iter().filter(|rule| applies(rule, features)) {
        allowed = rule["action"] == "allow";
    }
    allowed
}

fn applies(rule: &Value, features: &[&str]) -> bool {
    let os = &rule["os"];
    // `arch` only ever says "x86", meaning 32 bits. `version` is a regex; on Windows Mojang only
    // uses "^10\\.", which Windows 10 and 11 (both "10.0") match.
    os["name"].as_str().is_none_or(|name| name == "windows")
        && os["arch"].is_null()
        && os["version"].as_str().is_none_or(|v| "10.0".starts_with(&v.trim_start_matches('^').replace("\\.", ".")))
        && rule["features"].as_object().is_none_or(|wanted| {
            wanted.iter().all(|(feature, on)| *on == Value::Bool(features.contains(&feature.as_str())))
        })
}

/// `root` joined with a relative path taken from a Mojang file or a jar. Refuses anything that
/// could leave `root`.
pub fn under(root: &Path, relative: &str) -> Result<PathBuf> {
    let mut path = root.to_path_buf();
    for segment in relative.split(['/', '\\']).filter(|s| !s.is_empty()) {
        if segment == "." || segment == ".." || segment.contains(':') {
            return Err(format!("ruta no permitida: {relative}").into());
        }
        path.push(segment);
    }
    Ok(path)
}

/// A file Mojang lists with its SHA-1 and size, and where it goes on disk.
#[derive(Clone)]
pub struct Download {
    pub url: String,
    pub sha1: String,
    pub size: u64,
    pub path: PathBuf,
    /// A smaller LZMA copy to transfer instead, as (url, size). Java runtime files have one.
    pub lzma: Option<(String, u64)>,
}

impl Download {
    /// From a Mojang `{url, sha1, size}` entry.
    pub fn new(entry: &Value, path: PathBuf) -> Result<Self> {
        let url = entry["url"].as_str().ok_or("entrada de descarga sin url")?;
        let sha1 = entry["sha1"].as_str().ok_or("entrada de descarga sin sha1")?;
        Ok(Self { url: url.into(), sha1: sha1.into(), size: entry["size"].as_u64().unwrap_or(0), path, lzma: None })
    }

    /// Present means the right size, which is what Mojang's launcher checks too. The content
    /// was verified when it was downloaded.
    pub fn is_present(&self) -> bool {
        fs::metadata(&self.path).is_ok_and(|m| m.is_file() && (self.size == 0 || m.len() == self.size))
    }

    pub fn transfer_size(&self) -> u64 {
        self.lzma.as_ref().map_or(self.size, |(_, size)| *size)
    }

    fn fetch(&self) -> Result<()> {
        let bytes = match &self.lzma {
            Some((url, _)) => {
                let mut bytes = Vec::new();
                lzma_rs::lzma_decompress(&mut Cursor::new(get(url)?), &mut bytes)?;
                bytes
            }
            None => get(&self.url)?,
        };
        check_sha1(&bytes, &self.sha1, &self.url)?;
        save(&self.path, &bytes)
    }
}

pub struct Progress {
    pub bytes: u64,
    pub total_bytes: u64,
    pub files: usize,
    pub total_files: usize,
}

/// Fetches every download that is not present yet, several at a time, and stops at the first
/// file that still fails after a few attempts.
pub fn download_all(downloads: Vec<Download>, report: &(dyn Fn(Progress) + Sync)) -> Result<()> {
    let mut seen = HashSet::new();
    // Different names can share a file (assets with the same content): each is fetched once.
    let pending: Vec<Download> =
        downloads.into_iter().filter(|d| seen.insert(d.path.clone()) && !d.is_present()).collect();
    if pending.is_empty() {
        return Ok(());
    }
    let total_bytes = pending.iter().map(Download::transfer_size).sum();
    let total_files = pending.len();
    log!("descargando {total_files} archivos ({} MB)", total_bytes / 1_000_000);
    report(Progress { bytes: 0, total_bytes, files: 0, total_files });

    let next = AtomicUsize::new(0);
    let bytes = AtomicU64::new(0);
    let files = AtomicUsize::new(0);
    let failure = Mutex::new(None::<String>);
    thread::scope(|scope| {
        for _ in 0..THREADS.min(total_files) {
            scope.spawn(|| {
                while let Some(download) = pending.get(next.fetch_add(1, Ordering::Relaxed)) {
                    if failure.lock().unwrap_or_else(PoisonError::into_inner).is_some() {
                        break;
                    }
                    if let Err(e) = fetch_with_retries(download) {
                        failure.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert(e.to_string());
                        break;
                    }
                    let size = download.transfer_size();
                    report(Progress {
                        bytes: bytes.fetch_add(size, Ordering::Relaxed) + size,
                        total_bytes,
                        files: files.fetch_add(1, Ordering::Relaxed) + 1,
                        total_files,
                    });
                }
            });
        }
    });
    match failure.into_inner().unwrap_or_else(PoisonError::into_inner) {
        Some(e) => Err(e.into()),
        None => Ok(()),
    }
}

fn fetch_with_retries(download: &Download) -> Result<()> {
    for attempt in 1.. {
        match download.fetch() {
            Ok(()) => return Ok(()),
            Err(e) if attempt < ATTEMPTS => {
                log!("intento {attempt} fallido, {}: {e}", download.url);
                thread::sleep(Duration::from_secs(attempt));
            }
            Err(e) => return Err(format!("No se pudo descargar {}: {e}", download.url).into()),
        }
    }
    unreachable!()
}

fn sha1_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha1::digest(bytes))
}

fn check_sha1(bytes: &[u8], expected: &str, url: &str) -> Result<()> {
    let actual = sha1_hex(bytes);
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(format!("SHA-1 incorrecto para {url}: {actual}, se esperaba {expected}").into());
    }
    Ok(())
}

/// Writes then renames, so an interrupted download never leaves a file that looks complete.
pub fn save(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut partial = OsString::from(path);
    partial.push(".part");
    fs::write(&partial, bytes)?;
    fs::rename(&partial, path)?;
    Ok(())
}
