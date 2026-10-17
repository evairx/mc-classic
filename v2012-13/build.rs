//! Prepares everything the single executable carries:
//! - the Java agent in `agent/` and the bridge applet in `bridge/`, compiled to Java 8 bytecode as
//!   `classic-agent.jar` and `bridge.jar`. Needs a JDK that accepts `--release 8`; it uses
//!   `JAVA_HOME` when set, otherwise `javac` and `jar` from PATH;
//! - the original 2012 `Minecraft.exe` (project root, or the `MINECRAFT_EXE` variable), copied as
//!   is. It is Mojang's file, so it is not in the repository;
//! - that exe's own icon, so this one looks the same in Explorer and the taskbar, and a manifest
//!   for the Windows 10 dialogs (common controls 6, which the progress dialog needs);
//! - every file in `assets/`, the web the launcher shows as its news (`src/news.rs`).
//!
//! `cargo dist` (`.cargo/config.toml`) builds the exe to hand out, with the release profile of
//! Cargo.toml: optimized for size, stripped.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use sha1::{Digest, Sha1};

/// The 2012 launcher this project was built and tested against.
const MINECRAFT_EXE_SHA1: &str = "ce72c12d42135bf9951570f54f8c97d2cd9ea297";

const APP_MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0"
        processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true</dpiAware>
    </windowsSettings>
  </application>
</assembly>
"#;

fn main() {
    println!("cargo:rerun-if-changed=agent");
    println!("cargo:rerun-if-changed=assets");
    println!("cargo:rerun-if-changed=bridge");
    println!("cargo:rerun-if-changed=Minecraft.exe");
    println!("cargo:rerun-if-env-changed=MINECRAFT_EXE");
    println!("cargo:rerun-if-env-changed=JAVA_HOME");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    if env::var("PROFILE").is_ok_and(|profile| profile == "debug") {
        println!("cargo:warning=Build de prueba, sin optimizar. El Minecraft.exe para repartir se hace con `cargo dist`: target\\release\\Minecraft.exe");
    }

    build_jar(Path::new("agent/src"), &out.join("agent-classes"), &out.join("classic-agent.jar"), Some("agent/MANIFEST.MF"));
    build_jar(Path::new("bridge/src"), &out.join("bridge-classes"), &out.join("bridge.jar"), None);
    embed_assets(Path::new("assets"), &out.join("assets.rs"));

    let source = env::var_os("MINECRAFT_EXE").map_or_else(|| PathBuf::from("Minecraft.exe"), PathBuf::from);
    let minecraft = fs::read(&source).unwrap_or_else(|e| {
        panic!("no se pudo leer {} ({e}). Copia el Minecraft.exe original de 2012 a la raiz del proyecto o define MINECRAFT_EXE", source.display())
    });
    if format!("{:x}", Sha1::digest(&minecraft)) != MINECRAFT_EXE_SHA1 {
        println!("cargo:warning={} no es el launcher de 2012 conocido; puede que no funcione", source.display());
    }
    fs::write(out.join("Minecraft.exe"), &minecraft).expect("OUT_DIR");

    let icon = out.join("minecraft.ico");
    fs::write(&icon, original_icon(&minecraft)).expect("OUT_DIR");
    let manifest = out.join("app.manifest");
    fs::write(&manifest, APP_MANIFEST).expect("OUT_DIR");
    let rc = out.join("app.rc");
    let escaped = |path: &Path| path.display().to_string().replace('\\', "\\\\");
    // 24 is RT_MANIFEST; id 1 is the manifest Windows reads for an exe.
    fs::write(&rc, format!("1 ICON \"{}\"\n1 24 \"{}\"\n", escaped(&icon), escaped(&manifest))).expect("OUT_DIR");
    embed_resource::compile(&rc, embed_resource::NONE).manifest_optional().expect("compilar los recursos");
}

/// Compiles every .java file under `sources` to Java 8 bytecode and packs it into `jar`.
fn build_jar(sources: &Path, classes: &Path, jar: &Path, manifest: Option<&str>) {
    let _ = fs::remove_dir_all(classes);
    let mut files = Vec::new();
    files_under(sources, &mut files);
    files.retain(|path| path.extension().is_some_and(|ext| ext == "java"));
    run(Command::new(jdk_tool("javac"))
        .args(["--release", "8", "-Xlint:-options", "-encoding", "UTF-8", "-d"])
        .arg(classes)
        .args(&files));
    let mut command = Command::new(jdk_tool("jar"));
    command.arg("--create").arg("--file").arg(jar);
    if let Some(manifest) = manifest {
        command.args(["--manifest", manifest]);
    }
    run(command.arg("-C").arg(classes).arg("."));
}

fn files_under(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let path = entry.expect("leer el directorio").path();
        if path.is_dir() {
            files_under(&path, files);
        } else {
            files.push(path);
        }
    }
}

/// Writes `list` for `include!`: `&[(path relative to dir, include_bytes!(file)), ...]`.
fn embed_assets(dir: &Path, list: &Path) {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    files_under(dir, &mut files);
    files.sort();
    let mut code = String::from("&[\n");
    for file in files {
        let name = file.strip_prefix(dir).expect("dentro de assets").to_string_lossy().replace('\\', "/");
        code += &format!("    ({name:?}, include_bytes!({:?})),\n", root.join(&file).display().to_string());
    }
    code.push(']');
    fs::write(list, code).expect("OUT_DIR");
}

/// The first icon group of the exe, as an .ico file.
fn original_icon(exe: &[u8]) -> Vec<u8> {
    use pelite::{resources::FindError, PeFile};
    let resources = PeFile::from_bytes(exe).and_then(|pe| pe.resources()).expect("Minecraft.exe sin recursos");
    let (_, group) = resources
        .icons()
        .next()
        .expect("Minecraft.exe sin icono")
        .unwrap_or_else(|e: FindError| panic!("icono de Minecraft.exe ilegible: {e}"));
    let mut ico = Vec::new();
    group.write(&mut ico).expect("escribir el .ico");
    ico
}

fn jdk_tool(name: &str) -> PathBuf {
    match env::var_os("JAVA_HOME") {
        Some(home) => PathBuf::from(home).join("bin").join(name),
        None => PathBuf::from(name),
    }
}

fn run(command: &mut Command) {
    let program = command.get_program().to_string_lossy().into_owned();
    let status = command.status().unwrap_or_else(|e| {
        panic!("no se pudo ejecutar {program}: {e}. Instala un JDK que acepte --release 8 o define JAVA_HOME")
    });
    assert!(status.success(), "{program} fallo ({status})");
}
