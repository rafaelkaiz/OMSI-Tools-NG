//! Listing the maps of all configured game directories.

use std::fs;
use std::path::PathBuf;

use crate::config::{GameDirs, Source};
use crate::encoding;
use crate::paths;

/// A map found in `<game dir>/maps/<folder>/global.cfg`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapEntry {
    /// `[friendlyname]` of the map (falls back to the folder name).
    pub name: String,
    /// The installation the map lives in.
    pub source: Source,
    /// The map folder.
    pub dir: PathBuf,
}

impl MapEntry {
    /// `Name [neoOMSI]`-style label; maps of the main OMSI 2 dir carry no suffix.
    pub fn label(&self) -> String {
        match self.source {
            Source::Omsi2 => self.name.clone(),
            other => format!("{} [{}]", self.name, other.label()),
        }
    }
}

/// Reads the line following `header` (e.g. `[friendlyname]`) from a config file's text.
pub fn read_value(text: &str, header: &str) -> Option<String> {
    let mut it = text.lines().map(str::trim_end);
    while let Some(line) = it.next() {
        if line == header {
            return it.next().map(str::to_string);
        }
    }
    None
}

/// Lists the maps of every configured directory, sorted by name (case-insensitive).
pub fn list_maps(dirs: &GameDirs) -> Vec<MapEntry> {
    let mut maps = Vec::new();
    for (source, base) in dirs.iter() {
        let Some(maps_dir) = paths::resolve_existing(base, "maps") else {
            continue;
        };
        let Ok(read) = fs::read_dir(&maps_dir) else {
            continue;
        };
        for entry in read.filter_map(Result::ok) {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let Some(global) = paths::resolve_existing(&dir, "global.cfg") else {
                continue;
            };
            let folder = entry.file_name().to_string_lossy().into_owned();
            let name = encoding::read_text(&global)
                .ok()
                .and_then(|t| read_value(&t, "[friendlyname]"))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or(folder);
            maps.push(MapEntry {
                name,
                source: *source,
                dir,
            });
        }
    }
    maps.sort_by_key(|m| (m.name.to_lowercase(), m.source));
    maps
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn lists_maps_from_all_dirs_with_labels() {
        let tmp = tempfile::tempdir().unwrap();
        let omsi = tmp.path().join("omsi");
        let neo = tmp.path().join("neo");
        for (d, folder, name) in [(&omsi, "Berlin", "Berlin Spandau"), (&neo, "Alpha", "Alpha Map")] {
            let m = d.join("Maps").join(folder);
            fs::create_dir_all(&m).unwrap();
            fs::write(m.join("global.cfg"), format!("[friendlyname]\r\n{name}\r\n")).unwrap();
        }
        // A folder without global.cfg is ignored; a map without friendlyname uses its folder name.
        fs::create_dir_all(omsi.join("Maps/Empty")).unwrap();
        fs::create_dir_all(omsi.join("Maps/NoName")).unwrap();
        fs::write(omsi.join("Maps/NoName/global.cfg"), "[map]\n").unwrap();

        let dirs = GameDirs::new(vec![(Source::Omsi2, omsi), (Source::NeoOmsi, neo)]);
        let maps = list_maps(&dirs);
        let labels: Vec<_> = maps.iter().map(MapEntry::label).collect();
        assert_eq!(labels, vec!["Alpha Map [neoOMSI]", "Berlin Spandau", "NoName"]);
    }
}
