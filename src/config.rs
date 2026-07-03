use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::action::Action;

pub type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MappingConfig {
    pub button_byte: usize,
    pub side_mask: u8,
    pub extra_mask: u8,
    #[serde(default = "default_side_action")]
    pub side_action: String,
    #[serde(default = "default_extra_action")]
    pub extra_action: String,
}

fn default_side_action() -> String {
    "forward".to_string()
}

fn default_extra_action() -> String {
    "back".to_string()
}

#[derive(Clone, Debug)]
pub struct ResolvedMapping {
    pub button_byte: usize,
    pub side_mask: u8,
    pub extra_mask: u8,
    pub side_action: Action,
    pub extra_action: Action,
}

impl MappingConfig {
    pub fn resolve(&self) -> AppResult<ResolvedMapping> {
        let side_action: Action = self
            .side_action
            .parse()
            .map_err(|e: String| format!("invalid side_action '{}': {}", self.side_action, e))?;
        let extra_action: Action = self
            .extra_action
            .parse()
            .map_err(|e: String| format!("invalid extra_action '{}': {}", self.extra_action, e))?;
        Ok(ResolvedMapping {
            button_byte: self.button_byte,
            side_mask: self.side_mask,
            extra_mask: self.extra_mask,
            side_action,
            extra_action,
        })
    }
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
