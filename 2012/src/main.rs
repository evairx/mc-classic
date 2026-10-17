//! Minecraft Launcher 2012: the original 2012 Minecraft launcher, unmodified, working again,
//! with every Minecraft version.
//!
//! This single executable carries the original `Minecraft.exe`, a Java agent and a local server.
//! Opening it shows only the real launcher window:
//! 1. it unpacks the original Minecraft.exe and the agent into `%APPDATA%\mc-classic\2012`,
//!    and the first time downloads the Java 8 that Mojang publishes, so nothing has to be
//!    installed;
//! 2. it opens the launcher's JAR (inside Minecraft.exe) with that Java and the agent, which
//!    rewrites the launcher's dead `http://<host>/<path>` URLs to `https://127.0.0.1:25580/<host>/<path>`;
//! 3. it answers those endpoints over HTTPS and prepares the version of the selected profile
//!    (`profiles.rs`, edited in the launcher's options) from Mojang's current servers while the
//!    player logs in: in the launcher itself if it can run it (up to 1.5.2), otherwise through the
//!    bridge (`bridge.rs`);
//! 4. it exits once the game has started, or when the launcher closes: only the game is left.
//!
//! Static files live in `server/www/<host>/<path>`, so the tree reads like a mirror of the 2012
//! servers; the news are `assets/`, served from this exe (`news.rs`). There is no console:
//! messages go to `server/server.log`, errors to a dialog box.

#![windows_subsystem = "windows"]

/// Writes a line to `server/server.log`.
macro_rules! log {
    ($($arg:tt)*) => { $crate::write_log(format_args!($($arg)*)) };
}

mod bridge;
mod dialog;
mod game;
mod minecraft_download;
mod mojang;
mod news;
mod profiles;
mod runtime;
mod updater;
mod versions;

use std::{
    collections::hash_map::RandomState,
    env,
    error::Error,
    ffi::OsString,
    fmt, fs,
    hash::BuildHasher,
    io::{self, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener},
    path::{Path, PathBuf},
    process::Command,
    ptr,
    sync::{Arc, Mutex, OnceLock, PoisonError},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    extract::{Form, State},
    http::{header, HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use hyper_util::{rt::TokioIo, service::TowerToHyperService};
use rustls_pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
use md5::{Digest, Md5};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::{Mutex as AsyncMutex, OnceCell};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

/// The agent's default server address points here (`classic.server` in Agent.java).
const PORT: u16 = 25580;
/// Launcher build sent in the login form (`version=13`); anything else gets "Old version".
const LAUNCHER_VERSION: &str = "13";

const MINECRAFT_EXE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/Minecraft.exe"));
const AGENT_JAR: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/classic-agent.jar"));

static LOG: OnceLock<Mutex<fs::File>> = OnceLock::new();

/// Everything lives under `%APPDATA%\mc-classic\2012`, never in the real `%APPDATA%\.minecraft`.
#[derive(Clone)]
pub struct Dirs {
    /// The original Minecraft.exe and `profiles.json`.
    home: PathBuf,
    /// Agent, certificate, served files and logs.
    server: PathBuf,
    www: PathBuf,
    /// Mojang's Java runtimes.
    runtime: PathBuf,
    /// Downloads every version shares, in the layout of Mojang's `.minecraft`: `versions`,
    /// `libraries` and `assets`.
    store: PathBuf,
    /// `user.home` of the launcher: its `.minecraft` holds the launcher's `bin` and `lastlogin`.
    launcher: PathBuf,
    /// One folder per profile, `instances\<profile>`: `APPDATA` of the game, or `user.home` once
    /// the game runs inside the launcher. Profiles do not share worlds or options: a newer version
    /// converts worlds so that older ones cannot read them, and old versions crash on the options
    /// newer ones write.
    instances: PathBuf,
}

impl Dirs {
    fn instance(&self, profile: &profiles::Profile) -> PathBuf {
        self.instances.join(profile.folder())
    }

    fn profiles(&self) -> PathBuf {
        self.home.join("profiles.json")
    }
}

type Prepare = std::result::Result<Prepared, String>;

struct App {
    dirs: Dirs,
    profiles: Mutex<profiles::Profiles>,
    /// The last version prepared, by the profile's version, while it is the one selected.
    prepared: AsyncMutex<Option<(String, Arc<Prepare>)>>,
    /// The version selector's list, fetched once it is fetched without error.
    versions: OnceCell<String>,
    /// A newer release of this launcher, looked for once.
    newer: OnceCell<Option<updater::Release>>,
    /// The release the player chose to update to, installed once the launcher closes.
    update: Mutex<Option<updater::Release>>,
    bridge: Arc<bridge::Bridge>,
}

struct Prepared {
    /// The version's id, such as `1.21.4` for `latest-release`.
    id: String,
    /// `latestVersion` in the login reply. The launcher keeps it in `bin/version`.
    token: String,
    /// The version's JSON when it runs through the bridge rather than inside the launcher.
    bridged: Option<Value>,
}

impl App {
    fn profile(&self) -> profiles::Profile {
        self.profiles.lock().unwrap_or_else(PoisonError::into_inner).selected().clone()
    }

    /// Prepares the selected profile's version, unless it already is; logins and downloads wait
    /// for it. A failure is not kept, so the next login tries again.
    async fn prepared(&self) -> Arc<Prepare> {
        let version = self.profile().version;
        let mut cached = self.prepared.lock().await;
        if let Some((held, prepared)) = cached.as_ref() {
            if *held == version {
                return prepared.clone();
            }
        }
        let dirs = self.dirs.clone();
        let spec = version.clone();
        let result = tokio::task::spawn_blocking(move || {
            let result = prepare(&dirs, &spec).map_err(|e| e.to_string());
            if let Err(e) = &result {
                log!("ERROR al preparar Minecraft {spec}: {e}");
                error_box(&format!("No se pudo preparar Minecraft {spec}.\n\n{e}"));
            }
            result
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
        let result = Arc::new(result);
        if result.is_ok() {
            *cached = Some((version, result.clone()));
        }
        result
    }

    /// The version list, fetched the first time it is fetched without error.
    async fn version_list(&self) -> std::result::Result<&String, String> {
        self.versions
            .get_or_try_init(|| async {
                tokio::task::spawn_blocking(|| versions::list().map_err(|e| e.to_string()))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            })
            .await
    }

    /// A newer release of this launcher, if there is one. Test builds (`cargo run`) do not look.
    async fn newer(&self) -> Option<&updater::Release> {
        if cfg!(debug_assertions) {
            return None;
        }
        self.newer
            .get_or_init(|| async {
                match tokio::task::spawn_blocking(updater::newer).await {
                    Ok(Ok(newer)) => {
                        if let Some(release) = &newer {
                            log!("Hay una versión nueva: {} (esta es {}).", release.version, updater::CURRENT);
                        }
                        newer
                    }
                    Ok(Err(e)) => {
                        log!("No se pudo buscar actualizaciones: {e}");
                        None
                    }
                    Err(e) => {
                        log!("No se pudo buscar actualizaciones: {e}");
                        None
                    }
                }
            })
            .await
            .as_ref()
    }

    /// Prepares the selected version in the background, so it is ready by the time of login.
    fn prepare_soon(self: &Arc<Self>) {
        let app = self.clone();
        tokio::spawn(async move {
            let _ = app.prepared().await;
        });
    }
}

fn main() {
    // The release build aborts on a panic, so this is the one chance to tell the player.
    std::panic::set_hook(Box::new(|info| {
        log!("PANIC: {info}");
        error_box(&format!("Error interno de Minecraft Launcher Classic.\n\n{info}"));
    }));
    if let Err(e) = run() {
        log!("ERROR: {e}");
        error_box(&e.to_string());
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    // A fixed per-user folder lets this one file live anywhere. It is not %APPDATA%\.minecraft,
    // so a modern install and its worlds are never touched. Other launchers get their own
    // folders next to this one in mc-classic.
    let appdata = PathBuf::from(env::var_os("APPDATA").ok_or("la variable APPDATA no está definida")?);
    let home = appdata.join("mc-classic").join("2012");
    // Where earlier builds kept it.
    let old = [appdata.join("mc-classic").join("2012-13"), appdata.join("minecraft-launcher-classic")];
    let moved = old.iter().find(|old| move_old_folder(old, &home));
    let server = home.join("server");
    fs::create_dir_all(&server)?;
    let _ = LOG.set(Mutex::new(fs::File::create(server.join("server.log"))?));
    log!("Minecraft Launcher Classic {} ({}).", updater::CURRENT, if cfg!(debug_assertions) { "build de prueba" } else { "release" });
    updater::clean_up();
    if let Some(old) = moved {
        log!("Datos movidos de {} a {}", old.display(), home.display());
    }
    let dirs = Dirs {
        www: server.join("www"),
        server,
        runtime: home.join("runtime"),
        store: home.join("data").join(".minecraft"),
        launcher: home.join("launcher"),
        instances: home.join("instances"),
        home,
    };
    fs::create_dir_all(&dirs.instances)?;

    install(&dirs.home.join("Minecraft.exe"), MINECRAFT_EXE)?;
    install(&dirs.server.join("classic-agent.jar"), AGENT_JAR)?;

    let app = Arc::new(App {
        profiles: Mutex::new(profiles::Profiles::load(&dirs.profiles())),
        dirs,
        prepared: AsyncMutex::new(None),
        versions: OnceCell::new(),
        newer: OnceCell::new(),
        update: Mutex::new(None),
        bridge: Arc::default(),
    });
    // The launcher understands `-u=<user>`, `-p=<password>` and `--noupdate`.
    let launcher_args: Vec<OsString> = env::args_os().skip(1).collect();
    // The runtime ends with this line, and the port is free again for an updated copy.
    tokio::runtime::Runtime::new()?.block_on(serve(app.clone(), launcher_args))?;
    log!("Launcher cerrado.");
    let update = app.update.lock().unwrap_or_else(PoisonError::into_inner).take();
    if let Some(release) = update {
        if let Err(e) = updater::install(&release) {
            log!("ERROR al actualizar: {e}");
            error_box(&e.to_string());
        }
        updater::restart()?;
    }
    Ok(())
}

/// Moves the folder of earlier builds to its new place, once: its downloads, Java and worlds
/// stay. Returns whether it did.
fn move_old_folder(old: &Path, new: &Path) -> bool {
    if new.exists() || !old.is_dir() {
        return false;
    }
    new.parent().is_some_and(|parent| fs::create_dir_all(parent).is_ok()) && fs::rename(old, new).is_ok()
}

/// Writes a file this program owns unless it already holds these bytes. Skipping the write
/// matters while a launcher is open: Windows keeps its exe and the agent jar locked.
fn install(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if fs::read(path).is_ok_and(|current| current == bytes) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)
}

/// Gets the version ready for the launcher: in the launcher itself if its jar still has the
/// applet, otherwise through the bridge.
fn prepare(dirs: &Dirs, version: &str) -> Result<Prepared> {
    let meta = versions::resolve(&dirs.store, version)?;
    let id = meta["id"].as_str().ok_or("la versión no trae id")?.to_string();
    let client = mojang::client(&meta, &dirs.store)?;
    mojang::download_all(vec![client.clone()], &|_| {})?;
    let prepared = if !versions::is_modded(version) && minecraft_download::has_applet(&client.path)? {
        let token = minecraft_download::serve_game(&dirs.www, &dirs.store.join("libraries"), &meta, &client.path)?;
        Prepared { id, token, bridged: None }
    } else {
        Prepared { id, token: minecraft_download::serve_bridge(&dirs.www)?, bridged: Some(meta) }
    };
    // The launcher offers "New update available" when bin/version holds something else, which
    // happens when the profile picks another version or this exe brings a new bridge. Without
    // the file it just installs.
    let installed = dirs.launcher.join(".minecraft").join("bin").join("version");
    if fs::read(&installed).is_ok_and(|held| held != java_utf(&prepared.token)) {
        fs::remove_file(&installed)?;
        log!("bin/version era de otra versión: el launcher instalará {} sin preguntar.", prepared.id);
    }
    let place = if prepared.bridged.is_some() { "fuera del launcher" } else { "dentro del launcher" };
    log!("Minecraft {} listo ({place}).", prepared.id);
    Ok(prepared)
}

/// What Java's DataOutputStream.writeUTF writes for an ASCII string, as the launcher stores
/// bin/version.
fn java_utf(text: &str) -> Vec<u8> {
    let mut bytes = (text.len() as u16).to_be_bytes().to_vec();
    bytes.extend_from_slice(text.as_bytes());
    bytes
}

async fn serve(app: Arc<App>, launcher_args: Vec<OsString>) -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| "no se pudo instalar el proveedor TLS")?;
    let (cert, key) = ensure_certificate(&app.dirs.server)?;
    let tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![CertificateDer::from_pem_file(cert)?], PrivateKeyDer::from_pem_file(key)?)?;
    let tls = tokio_rustls::TlsAcceptor::from(Arc::new(tls));

    // Bind before opening the launcher, so a second copy stops here instead of at login.
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, PORT));
    let listener = TcpListener::bind(addr).map_err(|e| {
        format!("Minecraft Launcher Classic ya está abierto, o algo más usa el puerto {PORT}.\n\n{e}")
    })?;

    // Prepare the version now, while Java downloads or the player types their login, and fetch
    // the version list for the options window and any update.
    app.prepare_soon();
    tokio::spawn({
        let app = app.clone();
        async move {
            let _ = app.version_list().await;
            let _ = app.newer().await;
        }
    });
    let router = Router::new()
        .route("/login.minecraft.net/", post(login))
        .route("/classic/play", post(play))
        .route("/classic/status", get(status))
        .route("/classic/profiles", get(get_profiles).post(set_profiles))
        .route("/classic/versions", get(version_list))
        .route("/classic/started", post(started))
        .route("/classic/update", get(update_offer).post(update_accepted))
        .route("/mcupdate.tumblr.com/", get(news::first_page))
        .route("/mcupdate.tumblr.com/page/{page}", get(news::more))
        .route("/mcupdate.tumblr.com/{*path}", get(news::file))
        .fallback(static_file)
        .with_state(app.clone());

    let profile = app.profile();
    log!("Servidor local en https://{addr}. Perfil: {}, versión {}. Login en MODO PRUEBAS: acepta cualquier contraseña.", profile.name, profile.version);
    listener.set_nonblocking(true)?;
    let server = https(tokio::net::TcpListener::from_std(listener)?, tls, router);
    let dirs = app.dirs.clone();
    let flags = app.profiles.lock().unwrap_or_else(PoisonError::into_inner).launcher_flags.clone();
    let launcher = tokio::task::spawn_blocking(move || open_launcher(&dirs, &flags, &launcher_args));
    tokio::select! {
        result = server => result?,
        result = launcher => result??,
    }
    Ok(())
}

/// Answers HTTPS on `listener` with `router`, in HTTP/1.1: all the 2012 launcher and Java 8 speak.
async fn https(listener: tokio::net::TcpListener, tls: tokio_rustls::TlsAcceptor, router: Router) -> Result<()> {
    loop {
        let (stream, _) = listener.accept().await?;
        let (tls, router) = (tls.clone(), router.clone());
        tokio::spawn(async move {
            let Ok(stream) = tls.accept(stream).await else { return };
            let connection = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), TowerToHyperService::new(router));
            let _ = connection.await;
        });
    }
}

/// Opens the launcher with Mojang's Java 8, downloading it the first time, and waits until it
/// closes.
fn open_launcher(dirs: &Dirs, flags: &str, launcher_args: &[OsString]) -> Result<()> {
    let java = runtime::Runtime::plan(&dirs.runtime, runtime::LAUNCHER)?;
    if !java.downloads.is_empty() {
        let downloads = java.downloads.clone();
        let total: u64 = downloads.iter().map(mojang::Download::transfer_size).sum();
        let first_text = format!("Descargando Java 8 de los servidores de Mojang: 0 de {} MB", total / 1_000_000);
        let outcome = dialog::progress("Preparando Minecraft (solo la primera vez)", &first_text, move |report| {
            mojang::download_all(downloads, &|p| {
                let text = format!(
                    "Descargando Java 8 de los servidores de Mojang: {} de {} MB",
                    p.bytes / 1_000_000,
                    p.total_bytes / 1_000_000
                );
                report(p.bytes * 100 / p.total_bytes.max(1), text);
            })
        });
        match outcome {
            None => return Ok(()), // the player cancelled
            Some(result) => result.map_err(|e| format!("No se pudo descargar Java desde los servidores de Mojang.\n\n{e}"))?,
        }
        java.finish()?;
    }

    let mut agent = OsString::from("-javaagent:");
    agent.push(dirs.server.join("classic-agent.jar"));
    // The agent points user.home here, so the launcher's .minecraft is its own.
    let mut home = OsString::from("-Dclassic.home=");
    home.push(&dirs.launcher);
    let mut launcher = Command::new(&java.javaw);
    launcher
        // The profiles' launcher flags come after the defaults, so they win.
        .args(profiles::LAUNCHER_FLAGS.split_whitespace())
        .args(flags.split_whitespace())
        // Without Java2D's Direct3D pipeline, which makes the window flash black while resizing.
        // The 2012 launcher passed this too when it relaunched itself.
        .arg("-Dsun.java2d.d3d=false")
        .arg(home)
        .arg(agent)
        .arg("-cp")
        .arg(dirs.home.join("Minecraft.exe"))
        .arg("net.minecraft.MinecraftLauncher")
        .args(launcher_args)
        .current_dir(&dirs.home);
    game::play(launcher)?;
    Ok(())
}

fn write_log(line: fmt::Arguments) {
    if let Some(file) = LOG.get() {
        let mut file = file.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = writeln!(file, "{line}");
    }
}

fn error_box(text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    // SAFETY: both arguments are NUL-terminated UTF-16 buffers that outlive the call.
    unsafe {
        MessageBoxW(ptr::null_mut(), dialog::wide(text).as_ptr(), dialog::wide("Minecraft Launcher Classic").as_ptr(), MB_OK | MB_ICONERROR)
    };
}

/// Creates the self-signed certificate on first run. The agent trusts it and pins its key.
fn ensure_certificate(dir: &Path) -> Result<(PathBuf, PathBuf)> {
    let cert_path = dir.join("server-cert.pem");
    let key_path = dir.join("server-key.pem");
    if !cert_path.exists() || !key_path.exists() {
        let mut params = rcgen::CertificateParams::new(vec!["localhost".to_string()])?;
        params.subject_alt_names.push(rcgen::SanType::IpAddress(Ipv4Addr::LOCALHOST.into()));
        params.distinguished_name.push(rcgen::DnType::CommonName, "Minecraft Launcher Classic (solo local)");
        // ECDSA P-256: its 91-byte public key fits the launcher's 294-byte pin buffer.
        let key = rcgen::KeyPair::generate()?;
        let cert = params.self_signed(&key)?;
        fs::write(&cert_path, cert.pem())?;
        fs::write(&key_path, key.serialize_pem())?;
        log!("Certificado nuevo: {}", cert_path.display());
    }
    Ok((cert_path, key_path))
}

/// The launcher also sends `password`; test mode accepts any, so it is not even read.
#[derive(Deserialize)]
struct LoginForm {
    user: String,
    version: String,
}

/// Legacy login: replies `latestVersion:downloadTicket:userName:sessionId` like
/// login.minecraft.net did, or an error line the launcher shows as is.
async fn login(State(app): State<Arc<App>>, Form(form): Form<LoginForm>) -> String {
    let reply = if form.version != LAUNCHER_VERSION {
        "Old version".to_string()
    } else if !is_valid_username(&form.user) {
        "Bad login".to_string()
    } else {
        match &*app.prepared().await {
            Ok(prepared) => format!("{}:deprecated:{}:{}", prepared.token, form.user, session_id()),
            // No colon, or the launcher would read it as a successful login.
            Err(_) => format!("Minecraft {} no disponible", app.profile().version).replace(':', " "),
        }
    };
    let outcome = if reply.contains(':') { "OK" } else { reply.as_str() };
    log!("POST login usuario={:?} launcher={} -> {outcome}", form.user, form.version);
    reply
}

/// 2012 account names: 2 to 16 letters, digits or underscores.
pub fn is_valid_username(name: &str) -> bool {
    (2..=16).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Session token for test mode. Not cryptographically strong; the real Microsoft login will
/// hand out the Minecraft access token instead.
fn session_id() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
    let state = RandomState::new();
    format!("{:016x}{:016x}", state.hash_one(nanos), state.hash_one(nanos.rotate_left(64)))
}

/// Sent by the bridge applet. With "Play offline" the launcher has no session, and the
/// username may be the default "Player".
#[derive(Deserialize)]
struct PlayForm {
    user: String,
    #[serde(default)]
    session: String,
}

/// The bridge applet asks for its version to be installed and started.
async fn play(State(app): State<Arc<App>>, Form(form): Form<PlayForm>) -> (StatusCode, String) {
    log!("POST play usuario={:?}", form.user);
    if !is_valid_username(&form.user) {
        return (StatusCode::BAD_REQUEST, format!("El usuario \"{}\" no es válido.", form.user));
    }
    let meta = match &*app.prepared().await {
        Ok(Prepared { bridged: Some(meta), .. }) => meta.clone(),
        Ok(prepared) => return (StatusCode::CONFLICT, format!("Minecraft {} corre dentro del launcher.", prepared.id)),
        Err(e) => return (StatusCode::SERVICE_UNAVAILABLE, e.clone()),
    };
    let token = if !form.session.is_empty() && form.session.bytes().all(|b| b.is_ascii_hexdigit()) {
        form.session
    } else {
        session_id()
    };
    let profile = app.profile();
    let launch = bridge::Launch {
        meta,
        user: form.user,
        token,
        instance: app.dirs.instance(&profile),
        flags: profile.game_flags(),
        resolution: profile.resolution(),
        java: (!profile.java.is_empty()).then(|| PathBuf::from(&profile.java)),
    };
    app.bridge.start(app.dirs.clone(), launch);
    (StatusCode::OK, "ok".to_string())
}

async fn status(State(app): State<Arc<App>>) -> String {
    app.bridge.status_text()
}

/// The profiles, for the launcher's options (see `profiles::Profiles::to_text`).
async fn get_profiles(State(app): State<Arc<App>>) -> String {
    app.profiles.lock().unwrap_or_else(PoisonError::into_inner).to_text(&app.dirs.instances)
}

/// Saves the profiles edited in the launcher's options, and prepares the selected version.
async fn set_profiles(State(app): State<Arc<App>>, body: String) -> (StatusCode, String) {
    let profiles = match profiles::Profiles::from_text(&body) {
        Ok(profiles) => profiles,
        Err(e) => return (StatusCode::BAD_REQUEST, e),
    };
    if let Err(e) = profiles.save(&app.dirs.profiles()) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("No se pudieron guardar los perfiles: {e}"));
    }
    let selected = profiles.selected().clone();
    log!("Perfiles guardados. Perfil: {}, versión {}.", selected.name, selected.version);
    *app.profiles.lock().unwrap_or_else(PoisonError::into_inner) = profiles;
    app.prepare_soon();
    (StatusCode::OK, "ok".to_string())
}

/// Every version a profile can pick (see `versions::list`).
async fn version_list(State(app): State<Arc<App>>) -> (StatusCode, String) {
    match app.version_list().await {
        Ok(text) => (StatusCode::OK, text.clone()),
        Err(e) => {
            log!("Sin lista de versiones: {e}");
            (StatusCode::SERVICE_UNAVAILABLE, e)
        }
    }
}

/// `<this version>\t<newer version>` when a newer release is out, for the agent to offer it;
/// empty otherwise.
async fn update_offer(State(app): State<Arc<App>>) -> String {
    match app.newer().await {
        Some(release) => format!("{}\t{}", updater::CURRENT, release.version),
        None => String::new(),
    }
}

/// The player chose to update: it happens once the agent closes the launcher.
async fn update_accepted(State(app): State<Arc<App>>) -> (StatusCode, &'static str) {
    let Some(release) = app.newer().await.cloned() else {
        return (StatusCode::NOT_FOUND, "no hay actualización");
    };
    log!("Actualización a {} aceptada.", release.version);
    *app.update.lock().unwrap_or_else(PoisonError::into_inner) = Some(release);
    (StatusCode::OK, "ok")
}

/// The agent saw the game start inside the launcher: it needs nothing more from this program.
async fn started() -> &'static str {
    log!("Minecraft iniciado dentro del launcher; el servidor local se cierra.");
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        std::process::exit(0);
    });
    "ok"
}

/// Serves `www/<host>/<path>` with the ETag the launcher expects: the quoted MD5 of the file.
async fn static_file(State(app): State<Arc<App>>, method: Method, uri: Uri, headers: HeaderMap) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    if uri.path().starts_with(minecraft_download::URL_PREFIX) {
        if let Err(e) = &*app.prepared().await {
            log!("{method} 503 {} (faltan los archivos del juego: {e})", uri.path());
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    }
    let Some(path) = resolve(&app.dirs.www, uri.path()) else {
        log!("{method} 404 {} (ruta no permitida)", uri.path());
        return StatusCode::NOT_FOUND.into_response();
    };
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(_) => {
            log!("{method} 404 {}", uri.path());
            return StatusCode::NOT_FOUND.into_response();
        }
    };
    let etag = format!("\"{:x}\"", Md5::digest(&bytes));
    let not_modified = headers.get(header::IF_NONE_MATCH).is_some_and(|v| v.as_bytes() == etag.as_bytes());
    let status = if not_modified { StatusCode::NOT_MODIFIED } else { StatusCode::OK };
    log!("{method} {} {} ({} bytes)", status.as_u16(), uri.path(), bytes.len());
    // The launcher divides by the summed Content-Length of every file, so it is always sent,
    // even on 304, where it must be the full size rather than 0.
    let response_headers = [
        (header::ETAG, etag),
        (header::CONTENT_TYPE, content_type(&path).to_string()),
        (header::CONTENT_LENGTH, bytes.len().to_string()),
    ];
    if not_modified {
        (status, response_headers).into_response()
    } else {
        (status, response_headers, bytes).into_response()
    }
}

/// Maps a URL path to a file under `www`. Segments are limited to plain file-name characters,
/// so nothing can climb out of the folder.
fn resolve(www: &Path, url_path: &str) -> Option<PathBuf> {
    let mut path = www.to_path_buf();
    for segment in url_path.split('/').filter(|s| !s.is_empty()) {
        let plain = segment.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
        if !plain || segment.starts_with('.') {
            return None;
        }
        path.push(segment);
    }
    if url_path.ends_with('/') {
        path.push("index.html");
    }
    Some(path)
}

pub fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("jar") => "application/java-archive",
        Some("png") => "image/png",
        Some("xml") => "text/xml",
        _ => "application/octet-stream",
    }
}
