//! Profiles, as in the launcher's options window: each has a name, a username, a version, the
//! game's window size, Java and JVM flags, and its own folder in `instances\`. The launcher's
//! own flags are shared. Empty settings mean the defaults.
//! They live in `profiles.json`; the agent reads and writes them as tab-separated lines (see
//! `to_text` and `from_text`), which plain Java 8 parses without a JSON library.

use std::{fs, path::Path};

use serde::{Deserialize, Serialize};

use crate::{is_valid_username, Result};

/// What Mojang's launcher passes to the game by default, for a profile without its own flags.
pub const GAME_FLAGS: &str = "-Xmx2G -XX:+UnlockExperimentalVMOptions -XX:+UseG1GC -XX:G1NewSizePercent=20 \
     -XX:G1ReservePercent=20 -XX:MaxGCPauseMillis=50 -XX:G1HeapRegionSize=32M";
/// The heap Launch4j asked for (RCDATA of the original exe), before the launcher flags of the
/// profiles. MinecraftLauncher relaunches itself, without the agent, unless it gets more than
/// 511 MB.
pub const LAUNCHER_FLAGS: &str = "-Xms512m -Xmx1024m";
/// The newest release Mojang lists.
pub const LATEST_RELEASE: &str = "latest-release";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub name: String,
    /// Fills the login form; empty leaves whatever the launcher remembered.
    pub username: String,
    /// A Mojang version id, `latest-release`, `latest-snapshot` or `fabric:<id>`.
    pub version: String,
    /// JVM flags of the game; empty means `GAME_FLAGS`.
    pub game_flags: String,
    /// The game window's size, as `<width>x<height>`; empty leaves it to the game.
    #[serde(default)]
    pub resolution: String,
    /// A `javaw.exe` of the player's; empty means the one Mojang publishes for the version.
    #[serde(default)]
    pub java: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profiles {
    pub selected: String,
    pub launcher_flags: String,
    pub profiles: Vec<Profile>,
}

impl Default for Profiles {
    fn default() -> Self {
        Self {
            selected: "Default".into(),
            launcher_flags: String::new(),
            profiles: vec![Profile {
                name: "Default".into(),
                username: String::new(),
                version: LATEST_RELEASE.into(),
                game_flags: String::new(),
                resolution: String::new(),
                java: String::new(),
            }],
        }
    }
}

impl Profile {
    pub fn game_flags(&self) -> Vec<String> {
        let flags = if self.game_flags.is_empty() { GAME_FLAGS } else { &self.game_flags };
        flags.split_whitespace().map(String::from).collect()
    }

    /// The window size, if the profile sets one.
    pub fn resolution(&self) -> Option<(u32, u32)> {
        let (width, height) = self.resolution.split_once('x')?;
        Some((width.trim().parse().ok()?, height.trim().parse().ok()?))
    }

    /// Its folder in `instances\`: the name, without characters Windows refuses in file names.
    pub fn folder(&self) -> String {
        let name: String =
            self.name.chars().map(|c| if c.is_alphanumeric() || " ._-()".contains(c) { c } else { '_' }).collect();
        let name = name.trim().trim_end_matches('.');
        if name.is_empty() { "_".into() } else { name.into() }
    }
}

impl Profiles {
    /// The saved profiles, or the default one when there are none yet.
    pub fn load(path: &Path) -> Self {
        let Ok(bytes) = fs::read(path) else { return Self::default() };
        match serde_json::from_slice::<Self>(&bytes).map_err(|e| e.to_string()).and_then(Self::checked) {
            Ok(profiles) => profiles,
            Err(e) => {
                log!("{} no es válido ({e}); se usa el perfil por defecto.", path.display());
                Self::default()
            }
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    pub fn selected(&self) -> &Profile {
        self.profiles.iter().find(|p| p.name == self.selected).unwrap_or(&self.profiles[0])
    }

    /// `defaults` (the flags of empty settings), `launcher`, `instances`, `selected` and one
    /// `profile` line each, with the folder of every profile last.
    pub fn to_text(&self, instances: &Path) -> String {
        let mut text = format!(
            "defaults\t{GAME_FLAGS}\t{LAUNCHER_FLAGS}\nlauncher\t{}\ninstances\t{}\nselected\t{}\n",
            self.launcher_flags,
            instances.display(),
            self.selected
        );
        for p in &self.profiles {
            let folder = instances.join(p.folder());
            text += &format!(
                "profile\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                p.name,
                p.username,
                p.version,
                p.game_flags,
                p.resolution,
                p.java,
                folder.display()
            );
        }
        text
    }

    /// The agent's edit, in the format of `to_text`; the folders it sends back are ignored.
    pub fn from_text(text: &str) -> std::result::Result<Self, String> {
        let mut profiles = Self { selected: String::new(), launcher_flags: String::new(), profiles: Vec::new() };
        for line in text.lines() {
            let fields: Vec<&str> = line.split('\t').map(str::trim).collect();
            match fields.as_slice() {
                ["launcher", flags, ..] => profiles.launcher_flags = flags.to_string(),
                ["selected", name, ..] => profiles.selected = name.to_string(),
                ["profile", name, username, version, flags, resolution, java, ..] => profiles.profiles.push(Profile {
                    name: name.to_string(),
                    username: username.to_string(),
                    version: version.to_string(),
                    game_flags: flags.to_string(),
                    resolution: resolution.to_string(),
                    java: java.to_string(),
                }),
                _ => {}
            }
        }
        profiles.checked()
    }

    /// Refuses what would break the launcher or the folders; an unknown selection picks the first.
    fn checked(mut self) -> std::result::Result<Self, String> {
        if self.profiles.is_empty() {
            return Err("Hace falta al menos un perfil.".into());
        }
        let mut folders = Vec::new();
        for p in &self.profiles {
            if p.name.is_empty() {
                return Err("Cada perfil necesita un nombre.".into());
            }
            if !p.username.is_empty() && !is_valid_username(&p.username) {
                return Err(format!("\"{}\" no es un usuario válido: 2 a 16 letras, cifras o _.", p.username));
            }
            let plain = p.version.bytes().all(|b| b.is_ascii_alphanumeric() || b" ._:-".contains(&b));
            if p.version.is_empty() || !plain {
                return Err(format!("\"{}\" no es una versión de Minecraft.", p.version));
            }
            if !p.resolution.is_empty() && p.resolution().is_none_or(|(w, h)| w == 0 || h == 0) {
                return Err(format!("\"{}\" no es una resolución: escribe ancho y alto, como 854x480.", p.resolution));
            }
            // Windows file names ignore case, so two profiles must not share a folder that way.
            let folder = p.folder().to_lowercase();
            if folders.contains(&folder) {
                return Err(format!("Ya hay otro perfil llamado \"{}\".", p.name));
            }
            folders.push(folder);
        }
        if !self.profiles.iter().any(|p| p.name == self.selected) {
            self.selected = self.profiles[0].name.clone();
        }
        Ok(self)
    }
}
