//! Runs the versions the 2012 launcher cannot run itself (1.6 and later, and the oldest ones:
//! pre-classic, classic and indev) the way Mojang's current launcher does.
//!
//! The launcher still goes through its usual steps: it downloads `minecraft.jar` and loads
//! `net.minecraft.client.MinecraftApplet` from it, but for these versions that jar is the bridge
//! applet (`bridge/`). The applet asks for `/classic/play`; this module then installs the version
//! (its Java, libraries, natives and assets) and starts it as a separate process, while the applet
//! draws `/classic/status` as the launcher's "Updating Minecraft" screen. Once the game window
//! shows up, the applet closes the launcher, and with it this program: only the game is left.

use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    thread,
    time::{Duration, Instant},
};

use md5::{Digest, Md5};
use serde_json::Value;

use crate::{
    game,
    mojang::{self, Download},
    runtime::{self, Runtime},
    Dirs, Result,
};

const ASSETS_URL: &str = "https://resources.download.minecraft.net";
/// What Mojang's launcher passes to versions whose JSON predates `arguments` (before 1.13).
const LEGACY_JVM_ARGUMENTS: [&str; 6] = [
    "-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump",
    "-Djava.library.path=${natives_directory}",
    "-Dminecraft.launcher.brand=${launcher_name}",
    "-Dminecraft.launcher.version=${launcher_version}",
    "-cp",
    "${classpath}",
];
/// The launcher closes once the game window is up; it gives up waiting for it after this long.
const WINDOW_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Default)]
pub struct Bridge {
    status: Mutex<Status>,
}

/// What a start needs besides the folders: the version, the player and the profile.
pub struct Launch {
    pub meta: Value,
    pub user: String,
    pub token: String,
    /// The profile's folder: `APPDATA` of the game, whose `.minecraft` holds worlds and options.
    pub instance: PathBuf,
    /// The profile's JVM flags.
    pub flags: Vec<String>,
    /// The game window's size, if the profile sets one.
    pub resolution: Option<(u32, u32)>,
    /// The profile's own `javaw.exe`, instead of the one Mojang publishes for the version.
    pub java: Option<PathBuf>,
}

#[derive(Default)]
struct Status {
    state: State,
    percent: u64,
    description: String,
    subtask: String,
}

#[derive(Default, Clone, Copy, PartialEq)]
enum State {
    #[default]
    Idle,
    Working,
    Running,
    Failed,
}

impl Bridge {
    /// Installs and starts the version on a new thread, unless that has already begun.
    pub fn start(self: &Arc<Self>, dirs: Dirs, launch: Launch) {
        {
            let mut status = self.lock();
            if status.state != State::Idle {
                return;
            }
            status.state = State::Working;
        }
        let bridge = self.clone();
        thread::spawn(move || {
            if let Err(e) = bridge.launch(&dirs, &launch) {
                log!("ERROR al iniciar el juego: {e}");
                let mut status = bridge.lock();
                status.state = State::Failed;
                status.description = e.to_string().replace(['\r', '\n'], " ");
                status.subtask.clear();
            }
        });
    }

    /// `key=value` lines for the bridge applet.
    pub fn status_text(&self) -> String {
        let status = self.lock();
        let state = match status.state {
            State::Idle => "idle",
            State::Working => "working",
            State::Running => "running",
            State::Failed => "failed",
        };
        format!(
            "state={state}\npercent={}\ndescription={}\nsubtask={}\n",
            status.percent, status.description, status.subtask
        )
    }

    fn lock(&self) -> MutexGuard<'_, Status> {
        self.status.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn report(&self, description: &str, subtask: String, percent: u64) {
        let mut status = self.lock();
        status.description = description.to_string();
        status.subtask = subtask;
        status.percent = percent.min(100);
    }

    fn launch(&self, dirs: &Dirs, launch: &Launch) -> Result<()> {
        let Launch { meta, user, token, instance, flags, resolution, java } = launch;
        let game = instance.join(".minecraft");
        let id = meta["id"].as_str().ok_or("la versión no trae id")?;
        log!("Preparando Minecraft {id} fuera del launcher.");
        self.report("Determining packages to load", String::new(), 0);

        let component = meta["javaVersion"]["component"].as_str().unwrap_or(runtime::LAUNCHER);
        let runtime = match java {
            Some(_) => None,
            None => Some(Runtime::plan(&dirs.runtime, component)?),
        };
        let javaw = java.clone().or_else(|| runtime.as_ref().map(|r| r.javaw.clone())).ok_or("sin Java")?;
        let mut downloads = runtime.as_ref().map(|r| r.downloads.clone()).unwrap_or_default();
        let libraries_dir = dirs.store.join("libraries");
        let mut classpath = Vec::new();
        let mut natives = Vec::new();
        for library in mojang::libraries(meta)? {
            if let Some(jar) = mojang::artifact(library, &libraries_dir)? {
                if !classpath.contains(&jar.path) {
                    classpath.push(jar.path.clone());
                }
                downloads.push(jar);
            }
            if let Some(jar) = mojang::natives(library, &libraries_dir)? {
                natives.push((jar.path.clone(), excluded(library)));
                downloads.push(jar);
            }
        }
        let client = mojang::client(meta, &dirs.store)?;
        classpath.push(client.path.clone());
        downloads.push(client);
        let logging = &meta["logging"]["client"];
        let mut log_argument = None;
        if let (Some(file_id), Some(argument)) = (logging["file"]["id"].as_str(), logging["argument"].as_str()) {
            let config = Download::new(&logging["file"], mojang::under(&dirs.store.join("assets").join("log_configs"), file_id)?)?;
            log_argument = Some(argument.replace("${path}", &config.path.to_string_lossy()));
            downloads.push(config);
        }
        let assets = Assets::plan(meta, &dirs.store.join("assets"), &game)?;
        downloads.extend(assets.objects.iter().map(|(_, download)| download.clone()));

        mojang::download_all(downloads, &|p| {
            self.report(
                "Downloading packages",
                format!(
                    "Retrieving: {} of {} files ({} of {} MB)",
                    p.files,
                    p.total_files,
                    p.bytes / 1_000_000,
                    p.total_bytes / 1_000_000
                ),
                p.bytes * 90 / p.total_bytes.max(1),
            )
        })?;
        if let Some(runtime) = &runtime {
            runtime.finish()?;
        }

        self.report("Extracting downloaded packages", String::new(), 95);
        let natives_dir = dirs.store.join("versions").join(id).join("natives");
        fs::create_dir_all(&natives_dir)?;
        for (jar, excluded) in &natives {
            extract(jar, excluded, &natives_dir)?;
        }
        let game_assets = assets.lay_out()?;
        fs::create_dir_all(&game)?;

        self.report(&format!("Starting Minecraft {id}"), String::new(), 100);
        let uuid = offline_uuid(user);
        let path = |p: &Path| p.to_string_lossy().into_owned();
        let classpath = classpath.iter().map(|p| path(p)).collect::<Vec<_>>().join(";");
        let (width, height) = resolution.unwrap_or_default();
        let mut values: HashMap<&str, String> = HashMap::from([
            ("auth_player_name", user.to_string()),
            ("auth_uuid", uuid.clone()),
            ("auth_access_token", token.to_string()),
            ("auth_session", format!("token:{token}:{uuid}")),
            ("auth_xuid", "0".to_string()),
            ("clientid", "0".to_string()),
            // What Mojang's launcher sends today for every version, since all accounts are Microsoft ones.
            ("user_type", "msa".to_string()),
            ("user_properties", "{}".to_string()),
            ("version_name", id.to_string()),
            ("version_type", meta["type"].as_str().unwrap_or("release").to_string()),
            ("game_directory", path(&game)),
            ("assets_root", path(&assets.dir)),
            ("assets_index_name", assets.id.clone()),
            ("game_assets", path(&game_assets)),
            ("natives_directory", path(&natives_dir)),
            ("library_directory", path(&libraries_dir)),
            ("classpath", classpath),
            ("classpath_separator", ";".to_string()),
            ("launcher_name", "minecraft-launcher-classic".to_string()),
            ("launcher_version", env!("CARGO_PKG_VERSION").to_string()),
        ]);
        values.insert("resolution_width", width.to_string());
        values.insert("resolution_height", height.to_string());
        let features: &[&str] = if resolution.is_some() { &["has_custom_resolution"] } else { &[] };
        let main_class = meta["mainClass"].as_str().ok_or("la versión no trae mainClass")?;
        let jvm = arguments(meta, "jvm", features).unwrap_or_else(|| LEGACY_JVM_ARGUMENTS.map(String::from).to_vec());
        let game_arguments = arguments(meta, "game", features).unwrap_or_else(|| {
            let mut list: Vec<String> =
                meta["minecraftArguments"].as_str().unwrap_or_default().split_whitespace().map(String::from).collect();
            // 1.6 to 1.12 take the size as Mojang's launcher passed it then; older ones have no option.
            if resolution.is_some() && main_class == "net.minecraft.client.main.Main" {
                list.extend(["--width", "${resolution_width}", "--height", "${resolution_height}"].map(String::from));
            }
            list
        });

        let log_path = dirs.server.join("game.log");
        let log = fs::File::create(&log_path)?;
        let child = Command::new(&javaw)
            .args(flags)
            // Log4Shell: Mojang's log configurations already fix it; this covers log4j 2.10+ too.
            .arg("-Dlog4j2.formatMsgNoLookups=true")
            .args(jvm.iter().map(|arg| substitute(arg, &values)))
            .args(log_argument)
            .arg(main_class)
            .args(game_arguments.iter().map(|arg| substitute(arg, &values)))
            .current_dir(&game)
            // Versions that look for %APPDATA%\.minecraft find the profile's folder instead.
            .env("APPDATA", instance)
            .stdout(log.try_clone()?)
            .stderr(log)
            .spawn()
            .map_err(|e| format!("No se pudo abrir {}: {e}", javaw.display()))?;
        log!("Minecraft {id} iniciado con {} (pid {}).", javaw.display(), child.id());
        self.watch(child, id, log_path)
    }

    /// Waits for the game window. The game then runs on its own: it outlives the launcher and
    /// this program, and writes its output to `game.log`.
    fn watch(&self, mut child: Child, id: &str, log_path: PathBuf) -> Result<()> {
        let started = Instant::now();
        while !game::has_visible_window(child.id()) && started.elapsed() < WINDOW_TIMEOUT {
            if let Some(exit) = child.try_wait()? {
                log!("Minecraft {id} se cerró al iniciar ({exit}):\n{}", tail(&log_path));
                return Err(format!("Minecraft {id} se cerró al iniciar ({exit}). Detalles en {}", log_path.display()).into());
            }
            thread::sleep(Duration::from_millis(250));
        }
        log!("Ventana de Minecraft {id} abierta; el launcher se cierra.");
        self.lock().state = State::Running;
        Ok(())
    }
}

/// The asset index of a version and the objects it lists.
struct Assets {
    id: String,
    /// The shared `assets` folder, with `indexes` and `objects`.
    dir: PathBuf,
    objects: Vec<(String, Download)>,
    /// Older versions read the files by name rather than by hash, from a copy:
    /// `assets/virtual/<id>` (1.6 to 1.7.2) or `resources/` in the game folder (before 1.6).
    copy_to: Option<PathBuf>,
}

impl Assets {
    fn plan(meta: &Value, assets_dir: &Path, game_dir: &Path) -> Result<Self> {
        let entry = &meta["assetIndex"];
        let id = entry["id"].as_str().ok_or("la versión no trae assetIndex")?;
        let index_file = Download::new(entry, mojang::under(&assets_dir.join("indexes"), &format!("{id}.json"))?)?;
        mojang::download_all(vec![index_file.clone()], &|_| {})?;
        let index: Value = serde_json::from_slice(&fs::read(&index_file.path)?)?;

        let mut objects = Vec::new();
        for (name, object) in index["objects"].as_object().ok_or("índice de assets sin objects")? {
            let hash = object["hash"].as_str().filter(|h| h.len() == 40 && h.bytes().all(|b| b.is_ascii_hexdigit()));
            let hash = hash.ok_or_else(|| format!("hash no válido para el asset {name}"))?;
            let download = Download {
                url: format!("{ASSETS_URL}/{}/{hash}", &hash[..2]),
                sha1: hash.to_string(),
                size: object["size"].as_u64().unwrap_or(0),
                path: assets_dir.join("objects").join(&hash[..2]).join(hash),
                lzma: None,
            };
            objects.push((name.clone(), download));
        }
        let copy_to = if index["map_to_resources"] == true {
            Some(game_dir.join("resources"))
        } else if index["virtual"] == true {
            Some(mojang::under(&assets_dir.join("virtual"), id)?)
        } else {
            None
        };
        Ok(Self { id: id.to_string(), dir: assets_dir.to_path_buf(), objects, copy_to })
    }

    /// Makes the copy older versions need. Returns the folder `${game_assets}` points at.
    fn lay_out(&self) -> Result<PathBuf> {
        let Some(dir) = &self.copy_to else { return Ok(self.dir.clone()) };
        for (name, object) in &self.objects {
            let target = mojang::under(dir, name)?;
            if fs::metadata(&target).is_ok_and(|m| m.len() == object.size) {
                continue;
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&object.path, &target)?;
        }
        Ok(dir.clone())
    }
}

/// Path prefixes not to extract from a natives jar, such as `META-INF/`.
fn excluded(library: &Value) -> Vec<String> {
    library["extract"]["exclude"]
        .as_array()
        .map(|list| list.iter().filter_map(|p| p.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

fn extract(jar: &Path, excluded: &[String], dir: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(fs::File::open(jar)?)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.is_dir() || excluded.iter().any(|prefix| entry.name().starts_with(prefix.as_str())) {
            continue;
        }
        let target = mojang::under(dir, entry.name())?;
        if fs::metadata(&target).is_ok_and(|m| m.len() == entry.size()) {
            continue;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        mojang::save(&target, &bytes)?;
    }
    Ok(())
}

/// A modern version's `arguments.jvm` or `arguments.game`, keeping what the rules allow here with
/// these optional features on.
fn arguments(meta: &Value, kind: &str, features: &[&str]) -> Option<Vec<String>> {
    let mut list = Vec::new();
    for item in meta["arguments"][kind].as_array()? {
        let value = match item {
            Value::String(_) => item,
            _ if mojang::allowed_with(&item["rules"], features) => &item["value"],
            _ => continue,
        };
        match value {
            Value::String(arg) => list.push(arg.clone()),
            Value::Array(args) => list.extend(args.iter().filter_map(|a| a.as_str().map(String::from))),
            _ => {}
        }
    }
    Some(list)
}

/// Replaces each `${name}` that has a value; others stay as they are, as in Mojang's launcher.
fn substitute(arg: &str, values: &HashMap<&str, String>) -> String {
    let mut out = String::new();
    let mut rest = arg;
    while let Some(start) = rest.find("${") {
        let Some(end) = rest[start..].find('}').map(|end| start + end) else { break };
        out.push_str(&rest[..start]);
        match values.get(&rest[start + 2..end]) {
            Some(value) => out.push_str(value),
            None => out.push_str(&rest[start..=end]),
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// The UUID Minecraft itself gives an offline player: version 3, from "OfflinePlayer:<name>".
fn offline_uuid(name: &str) -> String {
    let mut bytes: [u8; 16] = Md5::digest(format!("OfflinePlayer:{name}")).into();
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The last lines of the game's output, without the XML Mojang's log configuration wraps around
/// each message.
fn tail(log: &Path) -> String {
    let text = String::from_utf8_lossy(&fs::read(log).unwrap_or_default()).into_owned();
    let lines: Vec<String> = text
        .lines()
        .map(|line| line.trim().trim_start_matches("<log4j:Message><![CDATA[").trim_end_matches("]]></log4j:Message>"))
        .filter(|line| !line.is_empty() && !line.starts_with('<'))
        .map(|line| line.chars().take(200).collect())
        .collect();
    lines[lines.len().saturating_sub(15)..].join("\n")
}
