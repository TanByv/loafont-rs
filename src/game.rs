use std::collections::HashSet;
use std::env;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::lpk::{Archive, Profile};

const GAME_FOLDER: &str = "Lost Ark";

#[derive(Debug)]
pub struct GamePaths {
    pub font: PathBuf,
    pub backup: PathBuf,
    pub info: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct ModInfo {
    mod_sha256: String,
}

pub fn paths(game_dir: &Path) -> GamePaths {
    GamePaths {
        font: game_dir.join("font.lpk"),
        backup: game_dir.join("font.lpk.bak"),
        info: game_dir.join("font.lpk.modinfo.json"),
    }
}

pub fn find_game_dir() -> Option<PathBuf> {
    for library in steam_libraries() {
        let game_dir = library
            .join("steamapps")
            .join("common")
            .join(GAME_FOLDER)
            .join("EFGame");
        if game_dir.join("font.lpk").is_file() {
            return Some(game_dir);
        }
    }
    None
}

pub fn current_is_modded(game_dir: &Path, legacy_scan: Option<bool>) -> Option<bool> {
    let paths = paths(game_dir);
    if !paths.font.is_file() {
        return None;
    }

    if let Some(recorded) = recorded_hash(&paths.info) {
        return sha256_file(&paths.font)
            .ok()
            .map(|current| current == recorded);
    }

    legacy_scan.filter(|modded| *modded)
}

pub fn install<F: FnMut(&str)>(
    mod_archive: &Path,
    game_dir: &Path,
    profile: Profile,
    modded_now: Option<bool>,
    mut log: F,
) -> Result<()> {
    let paths = paths(game_dir);
    if !paths.font.is_file() {
        bail!(
            "font.lpk was not found in {}. Select the EFGame folder that contains it.",
            game_dir.display()
        );
    }

    let archive = Archive::read(mod_archive, profile)
        .context("The selected file is not a readable font.lpk archive.")?;
    log(&format!(
        "Installing {} ({} entries)",
        mod_archive
            .file_name()
            .unwrap_or_default()
            .to_string_lossy(),
        archive.entries.len()
    ));

    match modded_now {
        Some(true) => {
            if !paths.backup.is_file() {
                bail!(
                    "The game's font.lpk is already modded, but no backup exists. \
                     Use 'Verify integrity of game files' in Steam, then try again."
                );
            }
            log("The game is already modded; keeping the existing backup.");
        }
        Some(false) => {
            if paths.backup.exists() {
                log("Refreshing the backup from the current original file.");
            }
            fs::copy(&paths.font, &paths.backup)
                .with_context(|| format!("Could not create {}", paths.backup.display()))?;
            log("Backed up the original font.lpk to font.lpk.bak");
        }
        None => {
            if paths.backup.exists() {
                log("Could not determine the mod state; keeping the existing backup.");
            } else {
                fs::copy(&paths.font, &paths.backup)
                    .with_context(|| format!("Could not create {}", paths.backup.display()))?;
                log("Backed up the original font.lpk to font.lpk.bak");
            }
        }
    }

    fs::copy(mod_archive, &paths.font)
        .with_context(|| format!("Could not write {}", paths.font.display()))?;
    let info = ModInfo {
        mod_sha256: sha256_file(&paths.font)?,
    };
    fs::write(&paths.info, serde_json::to_vec_pretty(&info)?)
        .with_context(|| format!("Could not write {}", paths.info.display()))?;
    log("Installed the modded font.lpk.");

    Ok(())
}

pub fn revert<F: FnMut(&str)>(game_dir: &Path, mut log: F) -> Result<()> {
    let paths = paths(game_dir);
    if !paths.backup.is_file() {
        bail!(
            "No font.lpk.bak backup was found. \
             Use 'Verify integrity of game files' in Steam to restore the original."
        );
    }

    fs::copy(&paths.backup, &paths.font)
        .with_context(|| format!("Could not restore {}", paths.font.display()))?;
    let _ = fs::remove_file(&paths.backup);
    let _ = fs::remove_file(&paths.info);
    log("Restored the original font.lpk from the backup.");
    Ok(())
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file =
        File::open(path).with_context(|| format!("Could not open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 1 << 20];

    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(to_hex(&hasher.finalize()))
}

fn recorded_hash(info: &Path) -> Option<String> {
    let data = fs::read(info).ok()?;
    let info: ModInfo = serde_json::from_slice(&data).ok()?;
    Some(info.mod_sha256)
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(text, "{byte:02x}").expect("writing to a String cannot fail");
    }
    text
}

fn steam_libraries() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    #[cfg(windows)]
    if let Some(path) = registry_steam_path() {
        candidates.push(path);
    }

    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".steam").join("steam"));
        candidates.push(home.join(".local").join("share").join("Steam"));
        candidates.push(
            home.join("Library")
                .join("Application Support")
                .join("Steam"),
        );
    }

    for variable in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Ok(value) = env::var(variable) {
            candidates.push(PathBuf::from(value).join("Steam"));
        }
    }

    let mut libraries = Vec::new();
    let mut seen = HashSet::new();

    for candidate in candidates {
        push_unique(&mut libraries, &mut seen, candidate);
    }

    let base = libraries.clone();
    for library in base {
        let vdf = library.join("steamapps").join("libraryfolders.vdf");
        if let Ok(text) = fs::read_to_string(vdf) {
            for path in parse_library_paths(&text) {
                push_unique(&mut libraries, &mut seen, path);
            }
        }
    }

    libraries
}

fn push_unique(libraries: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>, path: PathBuf) {
    if seen.insert(path.clone()) {
        libraries.push(path);
    }
}

fn parse_library_paths(vdf: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    for line in vdf.lines() {
        let mut parts = line.split('"');
        loop {
            if parts.next().is_none() {
                break;
            }
            let Some(key) = parts.next() else {
                break;
            };
            if parts.next().is_none() {
                break;
            }
            let Some(value) = parts.next() else {
                break;
            };

            if key.eq_ignore_ascii_case("path") {
                paths.push(PathBuf::from(value.replace("\\\\", "\\")));
            }
        }
    }

    paths
}

#[cfg(windows)]
fn registry_steam_path() -> Option<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Valve\Steam")
        .ok()?;
    let path: String = key.get_value("SteamPath").ok()?;
    Some(PathBuf::from(path))
}

#[cfg(test)]
#[path = "../tests/unit/game.rs"]
mod tests;
