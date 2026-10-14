use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DeleteMode {
    #[default]
    Never,
    Ask,
    MirrorTrash,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub sync_dir: PathBuf,
    pub listen_port: u16,
    pub device_name: String,
    #[serde(default)]
    pub auto_accept_pairing: bool,
    #[serde(default)]
    pub delete_mode: DeleteMode,
}

impl Config {
    pub fn new(sync_dir: PathBuf, listen_port: u16, device_name: String) -> Self {
        Self {
            sync_dir,
            listen_port,
            device_name,
            auto_accept_pairing: false,
            delete_mode: DeleteMode::Never,
        }
    }

    pub fn default_data_dir() -> PathBuf {
        directories::ProjectDirs::from("com", "lansync", "lansync")
            .map(|dirs| dirs.data_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from(".lansync"))
    }

    pub fn load_or_default(path: &Path, default_sync_dir: PathBuf) -> Result<Self> {
        if path.exists() {
            let content = fs::read_to_string(path)
                .with_context(|| format!("Failed to read configuration file: {:?}", path))?;
            let cfg: Config = serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse configuration JSON: {:?}", path))?;
            Ok(cfg)
        } else {
            let dev_name = hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| "LanSync Device".to_string());
            let cfg = Config::new(default_sync_dir, 42424, dev_name);
            cfg.save(path)?;
            Ok(cfg)
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_config_save_load() {
        let dir = tempdir().unwrap();
        let cfg_path = dir.path().join("config.json");
        let sync_dir = dir.path().join("LanSync");

        let cfg = Config::load_or_default(&cfg_path, sync_dir.clone()).unwrap();
        assert_eq!(cfg.listen_port, 42424);
        assert_eq!(cfg.sync_dir, sync_dir);
        assert_eq!(cfg.delete_mode, DeleteMode::Never);

        let mut cfg2 = cfg.clone();
        cfg2.listen_port = 50000;
        cfg2.delete_mode = DeleteMode::MirrorTrash;
        cfg2.save(&cfg_path).unwrap();

        let loaded = Config::load_or_default(&cfg_path, sync_dir).unwrap();
        assert_eq!(loaded.listen_port, 50000);
        assert_eq!(loaded.delete_mode, DeleteMode::MirrorTrash);
    }
}
