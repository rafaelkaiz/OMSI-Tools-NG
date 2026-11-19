//! Persisted settings and the set of configured game directories.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Which installation a directory (and therefore a map or file) belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Source {
    Omsi2,
    NeoOmsi,
    OpenOmsi,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Omsi2 => "OMSI 2",
            Source::NeoOmsi => "neoOMSI",
            Source::OpenOmsi => "openOMSI",
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// User settings, stored as TOML.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub omsi_dir: Option<PathBuf>,
    pub neo_omsi_dir: Option<PathBuf>,
    pub open_omsi_dir: Option<PathBuf>,
    /// Folder unused content is moved into by the cleanup (instead of deleting it).
    pub cleanup_dir: Option<PathBuf>,
    /// `Some(true)` dark, `Some(false)` light, `None` follow the system.
    pub dark_theme: Option<bool>,
    /// UI language code (`en`, `es`, `de`, `pl`, `cs`, `fr`); `None` = English.
    pub language: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid config file: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("cannot serialise config: {0}")]
    Serialize(#[from] toml::ser::Error),
}

impl Config {
    /// Default location of the config file (`<config dir>/OMSI-Tools-NG/config.toml`).
    pub fn default_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "OMSI-Tools-NG")
            .map(|d| d.config_dir().join("config.toml"))
    }

    /// Loads the config; a missing file yields the default (empty) config.
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(toml::from_str(&text)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    /// The configured directories that actually exist, in priority order.
    pub fn game_dirs(&self) -> GameDirs {
        let mut dirs = Vec::new();
        for (src, dir) in [
            (Source::Omsi2, &self.omsi_dir),
            (Source::NeoOmsi, &self.neo_omsi_dir),
            (Source::OpenOmsi, &self.open_omsi_dir),
        ] {
            if let Some(d) = dir {
                if !d.as_os_str().is_empty() && d.is_dir() {
                    dirs.push((src, d.clone()));
                }
            }
        }
        GameDirs { dirs }
    }
}

/// The ordered list of configured (existing) game directories.
///
/// A file referenced by a map counts as present if it exists in *any* of them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GameDirs {
    dirs: Vec<(Source, PathBuf)>,
}

impl GameDirs {
    pub fn new(dirs: Vec<(Source, PathBuf)>) -> Self {
        GameDirs { dirs }
    }

    pub fn iter(&self) -> impl Iterator<Item = &(Source, PathBuf)> {
        self.dirs.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.dirs.is_empty()
    }

    pub fn len(&self) -> usize {
        self.dirs.len()
    }

    pub fn path_of(&self, source: Source) -> Option<&Path> {
        self.dirs
            .iter()
            .find(|(s, _)| *s == source)
            .map(|(_, p)| p.as_path())
    }

    /// Does `relative` (OMSI backslash notation, any case) exist in any directory?
    pub fn exists_in_any(&self, relative: &str) -> bool {
        self.find(relative).is_some()
    }

    /// Resolves `relative` in the first directory that contains it.
    pub fn find(&self, relative: &str) -> Option<(Source, PathBuf)> {
        self.dirs
            .iter()
            .find_map(|(src, base)| crate::paths::resolve_existing(base, relative).map(|p| (*src, p)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("sub/config.toml");
        assert_eq!(Config::load(&file).unwrap(), Config::default());

        let cfg = Config {
            omsi_dir: Some(tmp.path().to_path_buf()),
            neo_omsi_dir: Some("/nonexistent/neo".into()),
            ..Default::default()
        };
        cfg.save(&file).unwrap();
        assert_eq!(Config::load(&file).unwrap(), cfg);

        // Non-existing directories are dropped from the game dirs.
        let dirs = cfg.game_dirs();
        assert_eq!(dirs.len(), 1);
        assert!(dirs.path_of(Source::NeoOmsi).is_none());
    }
}
