use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct MappingConfig {
    pub button_byte: usize,
    pub side_mask: u8,
    pub extra_mask: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedProfile {
    pub path: String,
    pub vid: u16,
    pub pid: u16,
    pub serial: Option<String>,
    pub usage_page: u16,
    pub usage: u16,
    pub interface_number: i32,
    pub mapping: MappingConfig,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AppConfig {
    pub profile: Option<SavedProfile>,
}

pub fn config_path() -> AppResult<PathBuf> {
    let mut base =
        dirs::config_dir().ok_or("unable to locate config directory for current user")?;
    base.push("chimera-mapper");
    Ok(base.join("config.json"))
}

pub fn ensure_parent_dir(path: &Path) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

pub fn load_config() -> AppResult<AppConfig> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(AppConfig::default());
    }
    let raw = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

pub fn save_config(config: &AppConfig) -> AppResult<()> {
    let path = config_path()?;
    ensure_parent_dir(&path)?;
    fs::write(path, serde_json::to_string_pretty(config)?)?;
    Ok(())
}
