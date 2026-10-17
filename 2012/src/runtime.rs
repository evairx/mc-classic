//! The Java that Mojang publishes for its own launcher, so nothing has to be installed. Each
//! version names its runtime in `javaVersion.component`: `jre-legacy` (Java 8) up to 1.16,
//! then Java 16, 17, 21 and 25. The 2012 launcher itself runs on `jre-legacy`.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::{
    mojang::{self, Download},
    Result,
};

const INDEX_URL: &str =
    "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";
const PLATFORM: &str = "windows-x64";
/// The 2012 launcher was built for Java 6 and runs the game as an applet: Java 8 is the newest
/// that suits it.
pub const LAUNCHER: &str = "jre-legacy";
/// Written last, holding the manifest's SHA-1, once every file is in place.
const MARKER: &str = ".installed";

pub struct Runtime {
    pub javaw: PathBuf,
    /// Files still to fetch; empty when the runtime is installed.
    pub downloads: Vec<Download>,
    marker: PathBuf,
    manifest_sha1: String,
}

impl Runtime {
    /// Lists what `component` still needs under `root\<component>`. Once installed, it does not
    /// touch the network, so the game also starts offline.
    pub fn plan(root: &Path, component: &str) -> Result<Self> {
        let dir = mojang::under(root, component)?;
        let javaw = dir.join("bin").join("javaw.exe");
        let marker = dir.join(MARKER);
        if marker.is_file() && javaw.is_file() {
            return Ok(Self { javaw, downloads: Vec::new(), marker, manifest_sha1: String::new() });
        }
        let index = mojang::get_json(INDEX_URL)?;
        let entry = &index[PLATFORM][component][0]["manifest"];
        let (Some(url), Some(sha1)) = (entry["url"].as_str(), entry["sha1"].as_str()) else {
            return Err(format!("Mojang no publica Java {component} para Windows de 64 bits.").into());
        };
        let manifest = mojang::get_verified_json(url, sha1)?;
        let mut downloads = Vec::new();
        for (name, file) in manifest["files"].as_object().ok_or("manifiesto de Java sin archivos")? {
            let path = mojang::under(&dir, name)?;
            match file["type"].as_str() {
                Some("directory") => fs::create_dir_all(&path)?,
                Some("file") => {
                    let mut download = Download::new(&file["downloads"]["raw"], path)?;
                    let lzma = &file["downloads"]["lzma"];
                    if let (Some(url), Some(size)) = (lzma["url"].as_str(), lzma["size"].as_u64()) {
                        download.lzma = Some((url.to_string(), size));
                    }
                    downloads.push(download);
                }
                _ => {} // symbolic links appear only in the Linux and macOS runtimes
            }
        }
        Ok(Self { javaw, downloads, marker, manifest_sha1: sha1.to_string() })
    }

    /// Call once the downloads are done: from then on the runtime counts as installed.
    pub fn finish(&self) -> io::Result<()> {
        if self.downloads.is_empty() {
            return Ok(());
        }
        fs::write(&self.marker, &self.manifest_sha1)
    }
}
