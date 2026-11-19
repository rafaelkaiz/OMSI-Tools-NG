//! Scanning a map for the files it references and reporting the missing ones.
//!
//! Everything here is strictly read-only. KEY RULE: a file referenced by a map counts as
//! present if it exists in *any* configured game directory (a neoOMSI map may use an object
//! that lives in the OMSI 2 directory). Tiles are the exception: they live inside the map's
//! own folder.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::config::{GameDirs, Source};
use crate::encoding;
use crate::maps::MapEntry;
use crate::paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FileKind {
    Tile,
    Texture,
    Sceneryobject,
    Spline,
    Human,
    Vehicle,
    Unknown,
}

impl FileKind {
    pub const ALL: [FileKind; 7] = [
        FileKind::Tile,
        FileKind::Texture,
        FileKind::Sceneryobject,
        FileKind::Spline,
        FileKind::Human,
        FileKind::Vehicle,
        FileKind::Unknown,
    ];

    pub fn label(self) -> &'static str {
        match self {
            FileKind::Tile => "Tiles",
            FileKind::Texture => "Textures",
            FileKind::Sceneryobject => "Sceneryobjects",
            FileKind::Spline => "Splines",
            FileKind::Human => "Humans",
            FileKind::Vehicle => "Vehicles",
            FileKind::Unknown => "Unknown file types",
        }
    }

    /// Classifies a content file by its extension.
    pub fn from_extension(path: &str) -> FileKind {
        let lower = path.trim().to_lowercase();
        match lower.rsplit('.').next() {
            Some("sco") => FileKind::Sceneryobject,
            Some("sli") => FileKind::Spline,
            Some("hum") => FileKind::Human,
            Some("ovh") | Some("bus") | Some("zug") => FileKind::Vehicle,
            Some("map") => FileKind::Tile,
            _ => FileKind::Unknown,
        }
    }
}

/// One referenced file with its verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// The path as written in the map (first spelling seen).
    pub path: String,
    pub kind: FileKind,
    /// Where the file was found (`None` = missing).
    pub found: Option<(Source, PathBuf)>,
    /// Map files referencing it (e.g. `tile_0_0.map`, `parklist_p.txt`), sorted, deduplicated.
    pub referenced_by: Vec<String>,
}

impl FileEntry {
    pub fn is_missing(&self) -> bool {
        self.found.is_none()
    }
}

#[derive(Debug, Clone)]
pub struct ScanReport {
    pub map_name: String,
    pub map_dir: PathBuf,
    pub source: Source,
    /// All referenced files, sorted by kind then path.
    pub entries: Vec<FileEntry>,
    /// Non-fatal problems (unreadable files, empty lines, ...).
    pub warnings: Vec<String>,
    /// `true` if the scan was cancelled; the report is then partial.
    pub cancelled: bool,
}

impl ScanReport {
    pub fn missing(&self) -> impl Iterator<Item = &FileEntry> {
        self.entries.iter().filter(|e| e.is_missing())
    }

    pub fn missing_of(&self, kind: FileKind) -> impl Iterator<Item = &FileEntry> {
        self.missing().filter(move |e| e.kind == kind)
    }

    pub fn total_of(&self, kind: FileKind) -> usize {
        self.entries.iter().filter(|e| e.kind == kind).count()
    }

    pub fn missing_count(&self) -> usize {
        self.missing().count()
    }
}

/// What a [`Progress`] update is about (lets a UI localise its own message).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Reading tile `done + 1` of `total`.
    Tile,
    /// Reading parklists, humans and AI lists.
    Lists,
    /// Cleanup: reading map `done + 1`; `message` is the map label.
    Map,
    /// Cleanup: checking references between folders.
    Refs,
    /// Cleanup: measuring unused folders.
    Measure,
    /// Cleanup apply: working on the folder named in `message`.
    Apply,
    Done,
}

#[derive(Debug, Clone)]
pub struct Progress {
    pub phase: Phase,
    pub done: usize,
    pub total: usize,
    pub message: String,
}

/// A raw reference found while parsing the map's files.
#[derive(Debug, Clone)]
pub(crate) struct RawRef {
    pub path: String,
    pub kind: FileKind,
    pub from: String,
}

/// Finds a file inside the map folder, ignoring case.
fn map_file(map_dir: &Path, name: &str) -> Option<PathBuf> {
    paths::resolve_existing(map_dir, name)
}

fn is_vehicle_path(line: &str) -> bool {
    FileKind::from_extension(line) == FileKind::Vehicle
}

/// Parses all files of a map and collects the references (no existence checks).
pub(crate) fn collect_refs(
    map_dir: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
    warnings: &mut Vec<String>,
) -> Vec<RawRef> {
    let mut refs: Vec<RawRef> = Vec::new();
    let mut tiles: Vec<String> = Vec::new(); // relative to the map dir
    let mut tile_from: HashMap<String, String> = HashMap::new();

    let read = |name: &str, warnings: &mut Vec<String>| -> Option<String> {
        let p = map_file(map_dir, name)?;
        match encoding::read_text(&p) {
            Ok(t) => Some(t),
            Err(e) => {
                warnings.push(format!("Could not read {name}: {e}"));
                None
            }
        }
    };

    // --- global.cfg: tiles and ground textures ---
    match read("global.cfg", warnings) {
        None => warnings.push("global.cfg not found".into()),
        Some(text) => {
            let lines = encoding::lines(&text);
            let mut i = 0;
            while i < lines.len() {
                match lines[i] {
                    "[map]" => {
                        // x, y, tile file
                        if let Some(t) = lines.get(i + 3).filter(|t| !t.is_empty()) {
                            if tile_from.insert(paths::key(t), "global.cfg".into()).is_none() {
                                tiles.push(t.to_string());
                            }
                        }
                        i += 3;
                    }
                    "[groundtex]" => {
                        for t in lines.iter().skip(i + 1).take(2).filter(|t| !t.is_empty()) {
                            refs.push(RawRef {
                                path: t.to_string(),
                                kind: FileKind::Texture,
                                from: "global.cfg".into(),
                            });
                        }
                        i += 2;
                    }
                    _ => {}
                }
                i += 1;
            }
        }
    }

    // --- chrono tiles ---
    let mut chrono_tiles: Vec<(String, String)> = Vec::new(); // (relative path, from)
    if let Some(chrono) = map_file(map_dir, "Chrono") {
        if let Ok(rd) = fs::read_dir(&chrono) {
            let mut dirs: Vec<_> = rd.filter_map(Result::ok).filter(|e| e.path().is_dir()).collect();
            dirs.sort_by_key(|e| e.file_name());
            for d in dirs {
                let name = d.file_name().to_string_lossy().into_owned();
                if paths::resolve_existing(&d.path(), "Chrono.cfg").is_none() {
                    continue;
                }
                let mut files: Vec<String> = fs::read_dir(d.path())
                    .into_iter()
                    .flatten()
                    .filter_map(Result::ok)
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|f| f.to_lowercase().ends_with(".map"))
                    .collect();
                files.sort();
                for f in files {
                    chrono_tiles.push((format!("Chrono\\{name}\\{f}"), format!("Chrono\\{name}")));
                }
            }
        }
    }

    // --- tiles ---
    let total = tiles.len() + chrono_tiles.len();
    let all_tiles: Vec<(String, String)> = tiles
        .iter()
        .map(|t| (t.clone(), tile_from[&paths::key(t)].clone()))
        .chain(chrono_tiles)
        .collect();
    for (n, (tile, from)) in all_tiles.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return refs;
        }
        progress(Progress {
            phase: Phase::Tile,
            done: n,
            total,
            message: format!("Reading tile {} of {}", n + 1, total),
        });
        refs.push(RawRef {
            path: tile.clone(),
            kind: FileKind::Tile,
            from: from.clone(),
        });
        let Some(text) = read(tile, warnings) else {
            continue; // missing tiles are reported by the existence check
        };
        let lines = encoding::lines(&text);
        let mut i = 0;
        while i < lines.len() {
            let skip = match lines[i] {
                "[object]" | "[attachObj]" | "[splineAttachement]" | "[spline]" | "[spline_h]" => Some(2),
                "[splineAttachement_repeater]" => Some(4),
                _ => None,
            };
            if let Some(skip) = skip {
                if let Some(p) = lines.get(i + skip).filter(|p| !p.is_empty()) {
                    refs.push(RawRef {
                        path: p.to_string(),
                        kind: FileKind::from_extension(p),
                        from: tile.replace('\\', "/"),
                    });
                }
                i += skip;
            }
            i += 1;
        }
    }
    progress(Progress {
        phase: Phase::Lists,
        done: total,
        total,
        message: "Reading parklists, humans and AI lists".into(),
    });

    // --- parklists (map root and chrono) ---
    let mut parklists: Vec<String> = Vec::new();
    for i in 0.. {
        let name = if i == 0 { "parklist_p.txt".to_string() } else { format!("parklist_p_{i}.txt") };
        if map_file(map_dir, &name).is_none() {
            if i == 0 {
                warnings.push("parklist_p.txt not found".into());
                continue;
            }
            break;
        }
        parklists.push(name);
    }
    for (dir, name) in chrono_files(map_dir, |n| {
        let n = n.to_lowercase();
        n.starts_with("parklist_p") && n.ends_with(".txt")
    }) {
        parklists.push(format!("{dir}\\{name}"));
    }
    for pl in parklists {
        let Some(text) = read(&pl, warnings) else { continue };
        for (ln, line) in encoding::lines(&text).into_iter().enumerate() {
            if line.is_empty() {
                warnings.push(format!("{pl}: line {} is empty", ln + 1));
                continue;
            }
            refs.push(RawRef {
                path: line.to_string(),
                kind: FileKind::from_extension(line),
                from: pl.replace('\\', "/"),
            });
        }
    }

    // --- humans / drivers ---
    for name in ["humans.txt", "drivers.txt"] {
        match read(name, warnings) {
            None => {
                if name == "humans.txt" {
                    warnings.push("humans.txt not found".into());
                }
            }
            Some(text) => {
                for (ln, line) in encoding::lines(&text).into_iter().enumerate() {
                    if line.is_empty() {
                        warnings.push(format!("{name}: line {} is empty", ln + 1));
                        continue;
                    }
                    refs.push(RawRef {
                        path: line.to_string(),
                        kind: FileKind::Human,
                        from: name.into(),
                    });
                }
            }
        }
    }

    // --- AI lists (map root and chrono updates) ---
    let mut ailists: Vec<String> = vec!["ailists.cfg".into()];
    for (dir, name) in chrono_files(map_dir, |n| n.eq_ignore_ascii_case("ailists_#upd.cfg")) {
        ailists.push(format!("{dir}\\{name}"));
    }
    for al in &ailists {
        let Some(text) = read(al, warnings) else {
            if al == "ailists.cfg" {
                warnings.push("ailists.cfg not found".into());
            }
            continue;
        };
        let from = al.replace('\\', "/");
        let lines = encoding::lines(&text);
        let mut i = 0;
        while i < lines.len() {
            match lines[i] {
                "[aigroup_2]" => {
                    i += 3; // name, then one more header line
                    while i < lines.len() && lines[i] != "[end]" {
                        let v = lines[i].trim_end_matches(|c: char| c.is_ascii_digit() || c == '\t' || c == ' ');
                        if !v.is_empty() {
                            refs.push(RawRef { path: v.to_string(), kind: FileKind::from_extension(v), from: from.clone() });
                        }
                        i += 1;
                    }
                }
                "[aigroup_depot]" => {
                    i += 1;
                }
                "[aigroup_depot_typgroup_2]" => {
                    if let Some(v) = lines.get(i + 1).filter(|v| is_vehicle_path(v)) {
                        refs.push(RawRef { path: v.to_string(), kind: FileKind::Vehicle, from: from.clone() });
                    }
                    i += 1;
                }
                _ => {}
            }
            i += 1;
        }
    }
    refs
}

/// Files of the `Chrono/<x>/` subfolders whose name satisfies `pred`: `(Chrono\x, file)`.
fn chrono_files(map_dir: &Path, pred: impl Fn(&str) -> bool) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Some(chrono) = map_file(map_dir, "Chrono") else { return out };
    let Ok(rd) = fs::read_dir(chrono) else { return out };
    let mut dirs: Vec<_> = rd.filter_map(Result::ok).filter(|e| e.path().is_dir()).collect();
    dirs.sort_by_key(|e| e.file_name());
    for d in dirs {
        let dn = d.file_name().to_string_lossy().into_owned();
        let mut files: Vec<String> = fs::read_dir(d.path())
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|f| pred(f))
            .collect();
        files.sort();
        for f in files {
            out.push((format!("Chrono\\{dn}"), f));
        }
    }
    out
}

/// Vehicles referenced by a `.zug` train file: alternating vehicle path / direction lines.
pub(crate) fn train_vehicles(text: &str) -> Vec<String> {
    let lines = encoding::lines(text);
    lines
        .chunks(2)
        .filter_map(|c| c.first())
        .filter(|l| l.contains('\\') && is_vehicle_path(l))
        .map(|l| l.to_string())
        .collect()
}

/// Scans a map.
///
/// `progress` is called from the calling thread; run this on a worker thread to keep a UI
/// responsive. Setting `cancel` stops the scan early and marks the report as cancelled.
pub fn scan_map(
    dirs: &GameDirs,
    map: &MapEntry,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> ScanReport {
    let mut warnings = Vec::new();
    let mut raw = collect_refs(&map.dir, cancel, progress, &mut warnings);

    // Expand trains: the vehicles listed inside present .zug files count as references too.
    let train_refs: Vec<RawRef> = raw
        .iter()
        .filter(|r| r.kind == FileKind::Vehicle && r.path.to_lowercase().ends_with(".zug"))
        .cloned()
        .collect();
    let mut seen = BTreeSet::new();
    for t in train_refs {
        if !seen.insert(paths::key(&t.path)) {
            continue;
        }
        if let Some((_, p)) = dirs.find(&t.path) {
            if let Ok(text) = encoding::read_text(&p) {
                for v in train_vehicles(&text) {
                    raw.push(RawRef { path: v, kind: FileKind::Vehicle, from: t.path.replace('\\', "/") });
                }
            }
        }
    }

    // Group by (kind, normalised path).
    let mut grouped: HashMap<(FileKind, String), (String, BTreeSet<String>)> = HashMap::new();
    for r in raw {
        let e = grouped
            .entry((r.kind, paths::key(&r.path)))
            .or_insert_with(|| (r.path.clone(), BTreeSet::new()));
        e.1.insert(r.from);
    }

    let mut entries: Vec<FileEntry> = grouped
        .into_iter()
        .map(|((kind, _), (path, from))| {
            let found = if kind == FileKind::Tile {
                paths::resolve_existing(&map.dir, &path).map(|p| (map.source, p))
            } else {
                dirs.find(&path)
            };
            if kind == FileKind::Unknown {
                warnings.push(format!("Unknown file type: {path}"));
            }
            FileEntry { path, kind, found, referenced_by: from.into_iter().collect() }
        })
        .collect();
    entries.sort_by(|a, b| (a.kind, a.path.to_lowercase()).cmp(&(b.kind, b.path.to_lowercase())));
    warnings.sort();
    warnings.dedup();

    ScanReport {
        map_name: map.name.clone(),
        map_dir: map.dir.clone(),
        source: map.source,
        entries,
        warnings,
        cancelled: cancel.load(Ordering::Relaxed),
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use std::fs;
    use std::path::Path;

    pub fn write(base: &Path, rel: &str, content: &str) {
        let p = base.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }

    /// OMSI 2 dir with some content, plus a neoOMSI dir with one map using OMSI 2 content.
    pub fn trees() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let omsi = tmp.path().join("OMSI 2");
        let neo = tmp.path().join("neoOMSI");

        write(&omsi, "Sceneryobjects/Shared/house.sco", "[mesh]\nhouse.o3d\n");
        write(&omsi, "Sceneryobjects/Unused/junk.sco", "x");
        write(&omsi, "Splines/Roads/road.sli", "x");
        write(&omsi, "Splines/OldRoads/old.sli", "x");
        write(&omsi, "Texture/Ground/grass.bmp", "x");
        write(&omsi, "Vehicles/Bus1/bus.bus", "x");
        write(&omsi, "Vehicles/Bus1/bus.ovh", "x");
        write(&omsi, "Vehicles/Spare/old.bus", "x");
        write(&omsi, "Vehicles/Anzeigen/a.txt", "x");
        write(&omsi, "Humans/Man/man.hum", "x");

        let map = neo.join("Maps/Test");
        write(&neo, "Maps/Test/global.cfg",
            "[friendlyname]\r\nTest Map\r\n\r\n[map]\r\n0\r\n0\r\ntile_0_0.map\r\n\r\n[map]\r\n1\r\n0\r\nmissing_tile.map\r\n\r\n[groundtex]\r\nTexture\\Ground\\grass.bmp\r\nTexture\\Ground\\gone.bmp\r\n");
        write(&neo, "Maps/Test/tile_0_0.map",
            "[object]\r\n12\r\nSceneryobjects\\shared\\HOUSE.sco\r\n0\r\n[object]\r\n13\r\nSceneryobjects\\Shared\\gone.sco\r\n\
             [spline]\r\n1\r\nSplines\\Roads\\road.sli\r\n[spline_h]\r\n2\r\nSplines\\Roads\\gone.sli\r\n\
             [splineAttachement_repeater]\r\n1\r\n2\r\n3\r\nSceneryobjects\\Shared\\house.sco\r\n");
        write(&neo, "Maps/Test/parklist_p.txt", "Sceneryobjects\\Shared\\house.sco\r\n\r\nSceneryobjects\\Shared\\parked.sco\r\n");
        write(&neo, "Maps/Test/humans.txt", "Humans\\Man\\man.hum\r\nHumans\\Man\\woman.hum\r\n");
        write(&neo, "Maps/Test/ailists.cfg",
            "[aigroup_2]\r\nGroupA\r\n0\r\nVehicles\\Bus1\\bus.bus\t3\r\nVehicles\\Bus1\\missing.ovh\t1\r\n[end]\r\n\
             [aigroup_depot_typgroup_2]\r\nVehicles\\Bus1\\bus.ovh\r\n");
        (tmp, omsi, map)
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use crate::maps::list_maps;

    fn scan_fixture(include_omsi: bool) -> ScanReport {
        let (tmp, omsi, map_dir) = trees();
        let neo = tmp.path().join("neoOMSI");
        let mut v = vec![];
        if include_omsi {
            v.push((Source::Omsi2, omsi));
        }
        v.push((Source::NeoOmsi, neo));
        let dirs = GameDirs::new(v);
        let _ = map_dir;
        let map = list_maps(&dirs).into_iter().find(|m| m.name == "Test Map").unwrap();
        assert_eq!(map.name, "Test Map");
        let cancel = AtomicBool::new(false);
        let r = scan_map(&dirs, &map, &cancel, &mut |_| {});
        drop(tmp);
        r
    }

    fn missing_paths(r: &ScanReport, k: FileKind) -> Vec<String> {
        r.missing_of(k).map(|e| e.path.clone()).collect()
    }

    #[test]
    fn objects_in_other_dir_count_as_present() {
        let r = scan_fixture(true);
        assert_eq!(missing_paths(&r, FileKind::Sceneryobject), vec!["Sceneryobjects\\Shared\\gone.sco", "Sceneryobjects\\Shared\\parked.sco"]);
        assert_eq!(missing_paths(&r, FileKind::Spline), vec!["Splines\\Roads\\gone.sli"]);
        assert_eq!(missing_paths(&r, FileKind::Texture), vec!["Texture\\Ground\\gone.bmp"]);
        assert_eq!(missing_paths(&r, FileKind::Tile), vec!["missing_tile.map"]);
        assert_eq!(missing_paths(&r, FileKind::Human), vec!["Humans\\Man\\woman.hum"]);
        assert_eq!(missing_paths(&r, FileKind::Vehicle), vec!["Vehicles\\Bus1\\missing.ovh"]);

        // house.sco is present (found in OMSI 2) although the map lives in neoOMSI, and it is
        // reported once even though it is referenced from a tile (twice) and the parklist.
        let house = r.entries.iter().find(|e| e.path.to_lowercase().ends_with("house.sco")).unwrap();
        assert_eq!(house.found.as_ref().unwrap().0, Source::Omsi2);
        assert_eq!(house.referenced_by, vec!["parklist_p.txt", "tile_0_0.map"]);
        assert!(r.warnings.iter().any(|w| w.contains("parklist_p.txt: line 2 is empty")));
    }

    #[test]
    fn without_omsi_dir_everything_from_it_is_missing() {
        let r = scan_fixture(false);
        assert!(r.missing_count() > 8);
        assert!(r.entries.iter().find(|e| e.path.to_lowercase().ends_with("house.sco")).unwrap().is_missing());
    }

    #[test]
    fn cancel_stops_early() {
        let (tmp, _omsi, _) = trees();
        let dirs = GameDirs::new(vec![(Source::NeoOmsi, tmp.path().join("neoOMSI"))]);
        let map = list_maps(&dirs).remove(0);
        let cancel = AtomicBool::new(true);
        let r = scan_map(&dirs, &map, &cancel, &mut |_| {});
        assert!(r.cancelled);
    }

    #[test]
    fn train_files_expand() {
        let t = "Vehicles\\Bus1\\bus.bus\r\n0\r\nVehicles\\Bus1\\bus.ovh\r\n1\r\nGroupName\r\n0\r\n";
        assert_eq!(train_vehicles(t), vec!["Vehicles\\Bus1\\bus.bus", "Vehicles\\Bus1\\bus.ovh"]);
    }
}
