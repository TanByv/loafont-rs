use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::fontmap::FontItem;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FontSetting {
    pub path: PathBuf,
    pub name: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub profile: Option<String>,
    pub language: Option<String>,
    pub fonts: Vec<FontSetting>,
    pub output: Option<PathBuf>,
    pub game_dir: Option<PathBuf>,
    pub fontmap_rows: Vec<FontItem>,
    pub archive: Option<PathBuf>,
}

fn path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("LOAFontStudio")
        .join("settings.json")
}

pub fn load() -> Settings {
    fs::read(path())
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_default()
}

pub fn save(settings: &Settings) {
    let path = path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(data) = serde_json::to_vec_pretty(settings) {
        let _ = fs::write(path, data);
    }
}
