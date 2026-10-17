//! Updates of this launcher. Each launcher in the repository has its own version.json and
//! releases on its own, tagged `<folder>-<version>`, such as `v2012-13-0.1.1`, with its
//! `Minecraft.exe` and that file's SHA-256 (`.github/workflows/release.yml`). This one reads its
//! version.json on main, the production branch; when it is newer and its release is out, the agent
//! asks the player (UpdateCheck.java). If they say yes, the launcher closes, the new exe replaces
//! this one, and it opens in its place.

use std::{env, fs, path::PathBuf, process::Command};

use crate::{dialog, mojang, Result};

/// The version of this build, from version.json (build.rs).
pub const CURRENT: &str = env!("CLASSIC_VERSION");
/// This launcher's folder in the repository: its version.json, and the start of its tags.
const FOLDER: &str = "v2012-13";
const VERSION_URL: &str = "https://raw.githubusercontent.com/evairx/mc-classic/main/v2012-13/version.json";
const DOWNLOADS: &str = "https://github.com/evairx/mc-classic/releases/download";

#[derive(Clone)]
pub struct Release {
    pub version: String,
    url: String,
    /// The SHA-256 of the file, as hex.
    sha256: String,
}

/// The release on main, if it is newer than this build and already published.
pub fn newer() -> Result<Option<Release>> {
    let version = mojang::get_json(VERSION_URL)?["v"].as_str().ok_or("version.json sin \"v\"")?.to_string();
    if numbers(&version) <= numbers(CURRENT) {
        return Ok(None);
    }
    let url = format!("{DOWNLOADS}/{FOLDER}-{version}/Minecraft.exe");
    // main gets the new version a few minutes before its release is built.
    let Ok(sha256) = mojang::get_text(&format!("{url}.sha256")) else {
        log!("La versión {version} todavía no está publicada.");
        return Ok(None);
    };
    let sha256 = sha256.split_whitespace().next().unwrap_or_default().to_string();
    Ok(Some(Release { version, url, sha256 }))
}

/// "0.1.10" as [0, 1, 10], so it sorts after "0.1.9".
fn numbers(version: &str) -> Vec<u64> {
    version.split('.').map(|n| n.parse().unwrap_or(0)).collect()
}

/// Downloads the release over this exe, keeping the old one as `.old` until the next start.
/// Windows lets a running exe be renamed, not overwritten.
pub fn install(release: &Release) -> Result<()> {
    let exe = env::current_exe()?;
    let text = format!("Descargando Minecraft Launcher Classic {}", release.version);
    let first_text = text.clone();
    let outcome = dialog::progress("Actualizando Minecraft Launcher Classic", &first_text, {
        let url = release.url.clone();
        move |report| {
            mojang::get_with_progress(&url, &|bytes, total| {
                report(bytes * 100 / total.max(1), format!("{text}: {} de {} KB", bytes / 1000, total / 1000));
            })
        }
    });
    let bytes = match outcome {
        None => return Err("Actualización cancelada.".into()),
        Some(result) => result.map_err(|e| format!("No se pudo descargar la actualización: {e}"))?,
    };
    let actual: String =
        ring::digest::digest(&ring::digest::SHA256, &bytes).as_ref().iter().map(|b| format!("{b:02x}")).collect();
    if !actual.eq_ignore_ascii_case(&release.sha256) {
        return Err(format!("La actualización descargada está dañada (SHA-256 {actual}, se esperaba {}).", release.sha256).into());
    }
    let new = sibling(&exe, "new");
    let old = sibling(&exe, "old");
    fs::write(&new, &bytes)?;
    let _ = fs::remove_file(&old);
    fs::rename(&exe, &old)?;
    if let Err(e) = fs::rename(&new, &exe) {
        let _ = fs::rename(&old, &exe);
        return Err(e.into());
    }
    log!("Actualizado a {}: {}", release.version, exe.display());
    Ok(())
}

/// Opens this exe again, the updated one if `install` ran, with the same command line.
pub fn restart() -> Result<()> {
    Command::new(env::current_exe()?).args(env::args_os().skip(1)).spawn()?;
    Ok(())
}

/// Removes the exe an update left behind.
pub fn clean_up() {
    if let Ok(exe) = env::current_exe() {
        let _ = fs::remove_file(sibling(&exe, "old"));
    }
}

/// `Minecraft.exe` as `Minecraft.exe.<extension>`.
fn sibling(exe: &std::path::Path, extension: &str) -> PathBuf {
    let mut name = exe.as_os_str().to_owned();
    name.push(".");
    name.push(extension);
    PathBuf::from(name)
}
