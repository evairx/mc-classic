//! The versions a profile can pick: Mojang's (releases, snapshots, betas and alphas), the latest
//! release or snapshot, and Fabric's modded ones (`fabric:<id>`). A Fabric version is Mojang's
//! JSON with Fabric's loader on top, merged here the way Mojang's launcher reads `inheritsFrom`.

use std::fs;

use serde_json::{json, Value};

use crate::{mojang, Result};

const FABRIC_META: &str = "https://meta.fabricmc.net/v2/versions";
const FABRIC: &str = "fabric:";

/// One line per version, `<spec>\t<label>\t<type>`, newest first as Mojang lists them, each
/// release followed by its Fabric version. The selector filters by type: `latest`, `release`,
/// `snapshot`, `old_beta`, `old_alpha` or `fabric`.
pub fn list() -> Result<String> {
    let manifest = mojang::manifest()?;
    let latest = |kind: &str| manifest["latest"][kind].as_str().unwrap_or_default().to_string();
    let mut text = format!(
        "latest-release\tLatest release ({})\tlatest\nlatest-snapshot\tLatest snapshot ({})\tsnapshot\n",
        latest("release"),
        latest("snapshot")
    );
    // Fabric lists every version it supports; the stable ones are releases.
    let fabric: Vec<String> = match mojang::get_json(&format!("{FABRIC_META}/game")) {
        Ok(list) => list
            .as_array()
            .into_iter()
            .flatten()
            .filter(|v| v["stable"] == true)
            .filter_map(|v| v["version"].as_str().map(String::from))
            .collect(),
        Err(e) => {
            log!("Sin versiones de Fabric: {e}");
            Vec::new()
        }
    };
    for version in manifest["versions"].as_array().ok_or("la lista de versiones no trae versions")? {
        let (Some(id), Some(kind)) = (version["id"].as_str(), version["type"].as_str()) else { continue };
        let label = match kind {
            "snapshot" => format!("{id} (snapshot)"),
            "old_beta" => format!("{id} (beta)"),
            "old_alpha" => format!("{id} (alpha)"),
            _ => id.to_string(),
        };
        text += &format!("{id}\t{label}\t{kind}\n");
        if kind == "release" && fabric.iter().any(|f| f == id) {
            text += &format!("{FABRIC}{id}\tFabric {id}\tfabric\n");
        }
    }
    Ok(text)
}

/// Whether the version needs a loader, so it cannot run inside the 2012 launcher.
pub fn is_modded(spec: &str) -> bool {
    spec.starts_with(FABRIC)
}

/// The JSON of the version a profile names, ready to install.
pub fn resolve(store: &std::path::Path, spec: &str) -> Result<Value> {
    if let Some(game) = spec.strip_prefix(FABRIC) {
        return fabric(store, game);
    }
    let id = match spec {
        "latest-release" => latest(store, "release")?,
        "latest-snapshot" => latest(store, "snapshot")?,
        id => id.to_string(),
    };
    mojang::version(store, &id)
}

/// The id Mojang lists as its latest release or snapshot. The last one seen is kept, so the game
/// still starts without a connection.
fn latest(store: &std::path::Path, kind: &str) -> Result<String> {
    let cache = store.join("versions").join(format!("latest-{kind}"));
    match mojang::manifest() {
        Ok(manifest) => {
            let id = manifest["latest"][kind].as_str().ok_or("la lista de versiones no trae latest")?.to_string();
            mojang::save(&cache, id.as_bytes())?;
            Ok(id)
        }
        Err(e) => fs::read_to_string(&cache)
            .map_err(|_| format!("No se pudo leer la lista de versiones de Mojang: {e}").into()),
    }
}

/// Mojang's version `game` with the newest stable Fabric loader. Fabric's JSON is kept, so the
/// game still starts without a connection.
fn fabric(store: &std::path::Path, game: &str) -> Result<Value> {
    let vanilla = mojang::version(store, game)?;
    let cache = mojang::under(&store.join("versions").join("fabric"), &format!("{game}.json"))?;
    let profile = match fabric_profile(game) {
        Ok(profile) => {
            mojang::save(&cache, &serde_json::to_vec(&profile)?)?;
            profile
        }
        Err(e) if cache.is_file() => {
            log!("Sin conexión con Fabric ({e}); uso la copia guardada de {game}.");
            serde_json::from_slice(&fs::read(&cache)?)?
        }
        Err(e) => return Err(format!("No se pudo leer Fabric para {game}: {e}").into()),
    };
    inherit(&profile, vanilla)
}

fn fabric_profile(game: &str) -> Result<Value> {
    let loaders = mojang::get_json(&format!("{FABRIC_META}/loader"))?;
    let loader = loaders
        .as_array()
        .and_then(|list| list.iter().find(|l| l["stable"] == true))
        .and_then(|l| l["version"].as_str())
        .ok_or("Fabric no publica un loader estable")?;
    log!("Fabric {loader} para Minecraft {game}");
    let mut profile = mojang::get_json(&format!("{FABRIC_META}/loader/{game}/{loader}/profile/json"))?;
    // Fabric lists no SHA-1 for its own loader and mappings; Maven keeps one next to each file.
    for library in profile["libraries"].as_array_mut().into_iter().flatten() {
        if library["sha1"].is_null() {
            let (_, url) = maven_file(library)?;
            let sha1 = mojang::get_text(&format!("{url}.sha1"))?;
            library["sha1"] = Value::String(sha1.trim().chars().take(40).collect());
        }
    }
    Ok(profile)
}

/// `parent` with `child`'s id, main class, libraries (first) and extra arguments. `jar` keeps
/// the parent's id, which names the game jar.
fn inherit(child: &Value, mut parent: Value) -> Result<Value> {
    let mut libraries = Vec::new();
    for library in child["libraries"].as_array().into_iter().flatten() {
        libraries.push(maven_library(library)?);
    }
    libraries.extend(parent["libraries"].as_array().cloned().unwrap_or_default());
    parent["jar"] = parent["id"].clone();
    parent["id"] = child["id"].clone();
    parent["mainClass"] = child["mainClass"].clone();
    parent["libraries"] = Value::Array(libraries);
    for kind in ["jvm", "game"] {
        if let (Some(extra), Some(list)) = (child["arguments"][kind].as_array(), parent["arguments"][kind].as_array_mut()) {
            list.extend(extra.iter().cloned());
        }
    }
    Ok(parent)
}

/// A Maven library (`name`, `url`, `sha1`, `size`) in Mojang's form, with `downloads.artifact`.
fn maven_library(library: &Value) -> Result<Value> {
    if !library["downloads"].is_null() {
        return Ok(library.clone());
    }
    let (path, url) = maven_file(library)?;
    Ok(json!({
        "name": library["name"],
        "downloads": { "artifact": { "path": path, "url": url, "sha1": library["sha1"], "size": library["size"] } },
    }))
}

/// The jar a Maven library names: its path in a repository, and its URL in the library's own.
fn maven_file(library: &Value) -> Result<(String, String)> {
    let name = library["name"].as_str().ok_or("librería sin name")?;
    let [group, artifact, version] = name.split(':').collect::<Vec<_>>()[..] else {
        return Err(format!("librería no válida: {name}").into());
    };
    let path = format!("{}/{artifact}/{version}/{artifact}-{version}.jar", group.replace('.', "/"));
    let base = library["url"].as_str().unwrap_or("https://libraries.minecraft.net/");
    let url = format!("{}/{path}", base.trim_end_matches('/'));
    Ok((path, url))
}
