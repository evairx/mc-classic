//! The files the 2012 launcher downloads from `s3.amazonaws.com/MinecraftDownload/`, kept in
//! `www/` under that same path. Two kinds of version fill it:
//! - versions that still ship `net.minecraft.client.MinecraftApplet` (up to 1.5.2, and the alphas
//!   and betas): their real game and LWJGL, repackaged in the 2012 layout, so the game runs inside
//!   the launcher window as it did then;
//! - every other version: the bridge applet as `minecraft.jar`, plus empty jars, because the
//!   launcher insists on downloading all five files. See `bridge.rs`.
//!
//! Nothing from Mojang ships with this project: every player fetches their own copy.

use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
};

use md5::{Digest, Md5};
use serde_json::Value;

use crate::{mojang, Result};

/// URL path of these files; `folder` is the same place on disk.
pub const URL_PREFIX: &str = "/s3.amazonaws.com/MinecraftDownload/";

/// Files the launcher requests, in the order it downloads them.
const FILES: [&str; 5] =
    ["lwjgl.jar", "jinput.jar", "lwjgl_util.jar", "minecraft.jar", "windows_natives.jar.lzma"];

/// Plain jars taken from the version's libraries: (served name, Maven group:artifact).
const LIBRARY_JARS: [(&str, &str); 3] = [
    ("lwjgl.jar", "org.lwjgl.lwjgl:lwjgl"),
    ("lwjgl_util.jar", "org.lwjgl.lwjgl:lwjgl_util"),
    ("jinput.jar", "net.java.jinput:jinput"),
];
const JUTILS: &str = "net.java.jutils:jutils";

/// What the folder holds, written last, so that anything else or an interrupted download is
/// prepared again. The leading dot keeps the server from serving it.
const MARKER: &str = ".version";

/// The 2012 launcher starts the game as this applet.
const APPLET_CLASS: &str = "net/minecraft/client/MinecraftApplet.class";

const BRIDGE_JAR: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/bridge.jar"));

fn folder(www: &Path) -> PathBuf {
    www.join("s3.amazonaws.com").join("MinecraftDownload")
}

fn holds(dir: &Path, token: &str) -> bool {
    fs::read_to_string(dir.join(MARKER)).is_ok_and(|held| held == token)
        && FILES.iter().all(|name| dir.join(name).is_file())
}

/// Whether the 2012 launcher can run this game jar itself.
pub fn has_applet(client: &Path) -> Result<bool> {
    Ok(zip::ZipArchive::new(fs::File::open(client)?)?.by_name(APPLET_CLASS).is_ok())
}

/// Fills the folder with a version the launcher runs itself. Returns its `latestVersion`.
pub fn serve_game(www: &Path, libraries_dir: &Path, meta: &Value, client: &Path) -> Result<String> {
    let version = meta["id"].as_str().ok_or("la versión no trae id")?.to_string();
    let dir = folder(www);
    if holds(&dir, &version) {
        return Ok(version);
    }
    let libraries = mojang::libraries(meta)?;
    let library = |coordinates: &str| -> Result<mojang::Download> {
        libraries
            .iter()
            .find(|lib| group_and_artifact(lib) == Some(coordinates))
            .map(|lib| mojang::artifact(lib, libraries_dir))
            .transpose()?
            .flatten()
            .ok_or_else(|| format!("la versión no incluye {coordinates}").into())
    };
    let mut downloads = Vec::new();
    for (_, coordinates) in LIBRARY_JARS {
        downloads.push(library(coordinates)?);
    }
    let jutils = library(JUTILS)?;
    downloads.push(jutils.clone());
    let mut natives = Vec::new();
    for lib in &libraries {
        natives.extend(mojang::natives(lib, libraries_dir)?);
    }
    downloads.extend(natives.iter().cloned());
    mojang::download_all(downloads, &|_| {})?;

    fs::create_dir_all(&dir)?;
    let _ = fs::remove_file(dir.join(MARKER));
    save(&dir, "minecraft.jar", &fs::read(client)?)?;
    for (file, coordinates) in LIBRARY_JARS {
        let mut jar = fs::read(library(coordinates)?.path)?;
        if file == "jinput.jar" {
            // The 2012 jinput.jar carried net.java.games.util.plugins; Mojang now ships it as
            // jutils. Without it LWJGL logs "Failed to initialise controllers".
            let mut entries = jar_entries(&jar)?;
            for (name, bytes) in jar_entries(&fs::read(&jutils.path)?)? {
                if !name.starts_with("META-INF/") {
                    entries.entry(name).or_insert(bytes);
                }
            }
            jar = build_jar(&entries)?;
        }
        save(&dir, file, &jar)?;
    }

    // The launcher only extracts natives that sit at the root of the jar.
    let mut root_files = BTreeMap::new();
    let mut seen = HashSet::new();
    for download in natives.iter().filter(|d| seen.insert(d.path.clone())) {
        for (name, bytes) in jar_entries(&fs::read(&download.path)?)? {
            if !name.contains('/') {
                root_files.entry(name).or_insert(bytes);
            }
        }
    }
    save(&dir, "windows_natives.jar.lzma", &lzma(&build_jar(&root_files)?)?)?;
    fs::write(dir.join(MARKER), &version)?;
    log!("Listo: Minecraft {version} preparado para el launcher en {}", dir.display());
    Ok(version)
}

/// Fills the folder with the bridge applet. Returns its `latestVersion`: the same for every
/// bridged version, so switching between them downloads nothing.
pub fn serve_bridge(www: &Path) -> Result<String> {
    let token = format!("bridge-{}", &format!("{:x}", Md5::digest(BRIDGE_JAR))[..8]);
    let dir = folder(www);
    if holds(&dir, &token) {
        return Ok(token);
    }
    fs::create_dir_all(&dir)?;
    let _ = fs::remove_file(dir.join(MARKER));
    // The launcher puts these on the classpath and extracts the natives; the bridge needs none.
    let empty = build_jar(&BTreeMap::from([("META-INF/MANIFEST.MF".to_string(), b"Manifest-Version: 1.0\r\n\r\n".to_vec())]))?;
    for (file, _) in LIBRARY_JARS {
        save(&dir, file, &empty)?;
    }
    save(&dir, "windows_natives.jar.lzma", &lzma(&empty)?)?;
    save(&dir, "minecraft.jar", BRIDGE_JAR)?;
    fs::write(dir.join(MARKER), &token)?;
    Ok(token)
}

fn save(dir: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    mojang::save(&dir.join(name), bytes)?;
    log!("-> {name} ({} bytes)", bytes.len());
    Ok(())
}

fn group_and_artifact(library: &Value) -> Option<&str> {
    let name = library["name"].as_str()?;
    name.rfind(':').map(|end| &name[..end])
}

/// Every file entry of a jar (directories skipped), keyed by path.
fn jar_entries(jar: &[u8]) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(jar))?;
    let mut files = BTreeMap::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        files.insert(entry.name().to_string(), bytes);
    }
    Ok(files)
}

fn build_jar(files: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in files {
        writer.start_file(name.as_str(), options)?;
        writer.write_all(bytes)?;
    }
    Ok(writer.finish()?.into_inner())
}

/// LZMA "alone" format with the size in the header, which the launcher's bundled decoder reads.
fn lzma(data: &[u8]) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let options = lzma_rs::compress::Options {
        unpacked_size: lzma_rs::compress::UnpackedSize::WriteToHeader(Some(data.len() as u64)),
    };
    lzma_rs::lzma_compress_with_options(&mut Cursor::new(data), &mut output, &options)?;
    Ok(output)
}
