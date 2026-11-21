//! Cleanup: find content folders that no map uses, preview them, and move them away.
//!
//! Ported from the cleanup module of OMSI-Tools: the top-level folders below `Sceneryobjects`,
//! `Splines` and `Vehicles` that are not referenced by any map are reported. Analysis is
//! read-only. [`apply`] never hard-deletes: folders are moved into a quarantine folder or sent
//! to the recycle bin.
//!
//! To stay on the safe side, a folder also counts as used when a used folder's text files
//! (`.sco`, `.sli`, `.ovh`, ...) mention it by path, and the maps of **all** configured
//! directories are taken into account, with a folder name counting as used in every directory.

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::config::{GameDirs, Source};
use crate::encoding;
use crate::maps::MapEntry;
use crate::paths;
use crate::scanner::{scan_map, Phase, Progress};

/// Root folders that are cleaned up.
pub const ROOTS: [&str; 3] = ["Sceneryobjects", "Splines", "Vehicles"];

/// Folders below the roots that are never reported (OMSI and the nEditor need them without map
/// references). Names are compared in lower case.
const PROTECTED: [(&str, &str); 8] = [
    ("vehicles", "announcements"),
    ("vehicles", "anzeigen"),
    ("sceneryobjects", "neditor"),
    ("sceneryobjects", "neditorgenerated"),
    ("sceneryobjects", "neditorthumbnails"),
    ("splines", "neditor"),
    ("splines", "neditorgenerated"),
    ("splines", "neditorthumbnails"),
];

/// Extensions of files that may reference other content by path.
const TEXT_EXTENSIONS: [&str; 10] = ["sco", "sli", "ovh", "bus", "zug", "hum", "cfg", "txt", "osc", "map"];

/// A top-level content folder no map uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub source: Source,
    /// Root folder, e.g. `Sceneryobjects`.
    pub root: String,
    /// Path relative to the game directory in OMSI notation, e.g. `Sceneryobjects\Author`.
    pub relative: String,
    pub path: PathBuf,
    pub file_count: u64,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct CleanupReport {
    pub candidates: Vec<Candidate>,
    pub maps_scanned: usize,
    pub warnings: Vec<String>,
    pub cancelled: bool,
}

/// What to do with the selected folders. There is deliberately no "delete" variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Move to `<dir>/<source label>/<relative path>`; existing targets are never overwritten.
    MoveTo(PathBuf),
    /// Send to the system recycle bin / trash.
    RecycleBin,
}

#[derive(Debug, Clone, Default)]
pub struct ActionOutcome {
    pub done: Vec<String>,
    pub failed: Vec<(String, String)>,
}

fn used_key(root: &str, folder: &str) -> (String, String) {
    (root.to_lowercase(), folder.to_lowercase())
}

/// `(root, folder)` pairs mentioned in a text, e.g. `Vehicles\MAN\x.ovh`.
fn mentioned_folders(text: &str) -> Vec<(String, String)> {
    let norm = text.replace('/', "\\").to_lowercase();
    let mut out = Vec::new();
    for root in ROOTS {
        let needle = format!("{}\\", root.to_lowercase());
        let mut rest = norm.as_str();
        while let Some(pos) = rest.find(&needle) {
            let after = &rest[pos + needle.len()..];
            let folder: String = after
                .chars()
                .take_while(|c| !matches!(c, '\\' | '\r' | '\n' | '\t'))
                .collect();
            if !folder.is_empty() && after[folder.len()..].starts_with('\\') {
                out.push((root.to_lowercase(), folder.trim().to_string()));
            }
            rest = after;
        }
    }
    out
}

/// Counts files and bytes below `dir`, and returns readable text-like files.
fn walk(dir: &Path, count: &mut u64, size: &mut u64, texts: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.filter_map(Result::ok) {
        let p = e.path();
        match e.file_type() {
            Ok(t) if t.is_dir() => walk(&p, count, size, texts),
            Ok(_) => {
                *count += 1;
                *size += e.metadata().map(|m| m.len()).unwrap_or(0);
                let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase());
                if ext.is_some_and(|x| TEXT_EXTENSIONS.contains(&x.as_str())) {
                    texts.push(p);
                }
            }
            Err(_) => {}
        }
    }
}

/// Top-level folders of `root` inside `base`.
fn root_folders(base: &Path, root: &str) -> Vec<(String, PathBuf)> {
    let Some(dir) = paths::resolve_existing(base, root) else { return vec![] };
    let mut v: Vec<_> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect();
    v.sort_by_key(|(n, _)| n.to_lowercase());
    v
}

/// Finds the unused content folders. Read-only.
///
/// `maps` should be all maps of all configured directories (see [`crate::maps::list_maps`]).
pub fn analyze(
    dirs: &GameDirs,
    maps: &[MapEntry],
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> CleanupReport {
    let mut report = CleanupReport::default();
    let mut used: HashSet<(String, String)> = HashSet::new();
    let total = maps.len() + 2;

    // 1. Everything the maps reference.
    for (i, map) in maps.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
            return report;
        }
        progress(Progress {
            phase: Phase::Map,
            done: i,
            total,
            message: map.label(),
        });
        let scan = scan_map(dirs, map, cancel, &mut |_| {});
        if scan.cancelled {
            report.cancelled = true;
            return report;
        }
        for w in &scan.warnings {
            report.warnings.push(format!("{}: {w}", map.label()));
        }
        for entry in &scan.entries {
            let Some((_, path)) = &entry.found else { continue };
            for (_, base) in dirs.iter() {
                let Some(rel) = paths::relative_backslash(base, path) else { continue };
                let segs: Vec<&str> = rel.split('\\').collect();
                if segs.len() >= 3 && ROOTS.iter().any(|r| r.eq_ignore_ascii_case(segs[0])) {
                    used.insert(used_key(segs[0], segs[1]));
                }
            }
        }
    }
    report.maps_scanned = maps.len();

    // 2. Folders mentioned by used folders' files (transitively).
    progress(Progress { phase: Phase::Refs, done: maps.len(), total, message: "Checking references between folders".into() });
    let mut folder_paths: Vec<((String, String), Source, PathBuf)> = Vec::new();
    for (source, base) in dirs.iter() {
        for root in ROOTS {
            for (name, path) in root_folders(base, root) {
                folder_paths.push((used_key(root, &name), *source, path));
            }
        }
    }
    let mut scanned: HashSet<PathBuf> = HashSet::new();
    loop {
        let mut grew = false;
        for (key, _, path) in &folder_paths {
            if !used.contains(key) || !scanned.insert(path.clone()) {
                continue;
            }
            let (mut c, mut s, mut texts) = (0, 0, Vec::new());
            walk(path, &mut c, &mut s, &mut texts);
            for t in texts {
                if let Ok(text) = encoding::read_text(&t) {
                    for m in mentioned_folders(&text) {
                        grew |= used.insert(m);
                    }
                }
            }
        }
        if !grew {
            break;
        }
    }

    // 3. Everything else is a candidate.
    progress(Progress { phase: Phase::Measure, done: total - 1, total, message: "Measuring unused folders".into() });
    for (key, source, path) in folder_paths {
        if used.contains(&key) || PROTECTED.contains(&(key.0.as_str(), key.1.as_str())) {
            continue;
        }
        let base = dirs.path_of(source).expect("source comes from dirs");
        let relative = paths::relative_backslash(base, &path).unwrap_or_default();
        let root = ROOTS
            .iter()
            .find(|r| r.eq_ignore_ascii_case(&key.0))
            .copied()
            .unwrap_or_default()
            .to_string();
        let (mut c, mut s, mut texts) = (0, 0, Vec::new());
        walk(&path, &mut c, &mut s, &mut texts);
        report.candidates.push(Candidate { source, root, relative, path, file_count: c, size_bytes: s });
    }
    report
        .candidates
        .sort_by(|a, b| (a.root.as_str(), a.relative.to_lowercase(), a.source).cmp(&(b.root.as_str(), b.relative.to_lowercase(), b.source)));
    report.warnings.sort();
    report.warnings.dedup();
    progress(Progress { phase: Phase::Done, done: total, total, message: "Done".into() });
    report
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)? {
        let e = e?;
        let target = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &target)?;
        } else {
            fs::copy(e.path(), &target)?;
        }
    }
    Ok(())
}

/// Checks that a candidate is a real top-level content folder of a configured directory.
fn validate(dirs: &GameDirs, c: &Candidate) -> Result<(), String> {
    let base = dirs.path_of(c.source).ok_or("game directory is no longer configured")?;
    let path = fs::canonicalize(&c.path).map_err(|e| format!("folder not accessible: {e}"))?;
    let base = fs::canonicalize(base).map_err(|e| e.to_string())?;
    let rel = path.strip_prefix(&base).map_err(|_| "folder is outside the game directory".to_string())?;
    let comps: Vec<_> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    if comps.len() != 2 || !ROOTS.iter().any(|r| r.eq_ignore_ascii_case(&comps[0])) {
        return Err("not a top-level folder of Sceneryobjects, Splines or Vehicles".into());
    }
    Ok(())
}

/// Moves the selected folders away (see [`Action`]). Per-folder failures are collected, not fatal.
pub fn apply(dirs: &GameDirs, selected: &[Candidate], action: &Action, progress: &mut dyn FnMut(Progress)) -> ActionOutcome {
    let mut out = ActionOutcome::default();

    if let Action::MoveTo(dest) = action {
        // Moving into a folder we are about to move would never terminate sensibly.
        let dest_abs = fs::canonicalize(dest).unwrap_or_else(|_| dest.clone());
        for c in selected {
            if let Ok(p) = fs::canonicalize(&c.path) {
                if dest_abs.starts_with(&p) {
                    out.failed.push((c.relative.clone(), "destination lies inside the folder to move".into()));
                    return out;
                }
            }
        }
    }

    let mut seen = BTreeSet::new();
    for (i, c) in selected.iter().enumerate() {
        progress(Progress { phase: Phase::Apply, done: i, total: selected.len(), message: format!("{} ({})", c.relative, c.source) });
        if !seen.insert(c.path.clone()) {
            continue;
        }
        let label = format!("{} [{}]", c.relative, c.source);
        if let Err(e) = validate(dirs, c) {
            out.failed.push((label, e));
            continue;
        }
        let result: Result<(), String> = match action {
            Action::RecycleBin => trash::delete(&c.path).map_err(|e| e.to_string()),
            Action::MoveTo(dest) => {
                let mut target = dest.join(c.source.label());
                target.extend(c.relative.split('\\'));
                if target.exists() {
                    Err(format!("target already exists: {}", target.display()))
                } else {
                    (|| {
                        if let Some(p) = target.parent() {
                            fs::create_dir_all(p).map_err(|e| e.to_string())?;
                        }
                        if fs::rename(&c.path, &target).is_ok() {
                            return Ok(());
                        }
                        // Different volume: copy first, remove the original only after a full copy.
                        copy_dir(&c.path, &target).map_err(|e| format!("copy failed: {e}"))?;
                        fs::remove_dir_all(&c.path).map_err(|e| format!("copied, but removing the original failed: {e}"))
                    })()
                }
            }
        };
        match result {
            Ok(()) => out.done.push(label),
            Err(e) => out.failed.push((label, e)),
        }
    }
    progress(Progress { phase: Phase::Done, done: selected.len(), total: selected.len(), message: "Done".into() });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maps::list_maps;
    use crate::scanner::fixtures::{trees, write};

    fn setup() -> (tempfile::TempDir, GameDirs) {
        let (tmp, omsi, _) = trees();
        let neo = tmp.path().join("neoOMSI");
        // Referenced only indirectly by a used sceneryobject's text.
        write(&omsi, "Sceneryobjects/Shared/house.sco", "[mesh]\nhouse.o3d\nSplines\\Indirect\\spline.sli\n");
        write(&omsi, "Splines/Indirect/spline.sli", "x");
        // nEditor folders are never reported.
        for root in ["Sceneryobjects", "Splines"] {
            for name in ["nEditor", "nEditorGenerated", "nEditorThumbnails"] {
                write(&omsi, &format!("{root}/{name}/x.txt"), "x");
            }
        }
        let dirs = GameDirs::new(vec![(Source::Omsi2, omsi), (Source::NeoOmsi, neo)]);
        (tmp, dirs)
    }

    #[test]
    fn finds_unused_folders_across_dirs() {
        let (_tmp, dirs) = setup();
        let maps = list_maps(&dirs);
        let r = analyze(&dirs, &maps, &AtomicBool::new(false), &mut |_| {});
        let rel: Vec<_> = r.candidates.iter().map(|c| c.relative.clone()).collect();
        // Shared (via neoOMSI map), Roads, Bus1 are used; Indirect via house.sco's text;
        // Anzeigen and the nEditor folders are protected.
        assert_eq!(rel, vec!["Sceneryobjects\\Unused", "Splines\\OldRoads", "Vehicles\\Spare"]);
        assert!(r.candidates.iter().all(|c| c.file_count == 1 && c.size_bytes == 1));
        assert_eq!(r.maps_scanned, 1);
    }

    #[test]
    fn analyze_changes_nothing_and_apply_moves() {
        let (tmp, dirs) = setup();
        let maps = list_maps(&dirs);
        let r = analyze(&dirs, &maps, &AtomicBool::new(false), &mut |_| {});
        assert!(r.candidates.iter().all(|c| c.path.exists()));

        let dest = tmp.path().join("quarantine");
        let out = apply(&dirs, &r.candidates[..1], &Action::MoveTo(dest.clone()), &mut |_| {});
        assert!(out.failed.is_empty(), "{:?}", out.failed);
        assert!(!r.candidates[0].path.exists());
        assert!(dest.join("OMSI 2/Sceneryobjects/Unused/junk.sco").is_file());
        // The others are untouched.
        assert!(r.candidates[1].path.exists());

        // Moving again to an occupied target never overwrites.
        let again = Candidate { path: r.candidates[1].path.clone(), ..r.candidates[1].clone() };
        write(&dest, "OMSI 2/Splines/OldRoads/x.txt", "keep");
        let out = apply(&dirs, &[again], &Action::MoveTo(dest.clone()), &mut |_| {});
        assert_eq!(out.failed.len(), 1);
        assert!(r.candidates[1].path.exists());
    }

    #[test]
    fn apply_refuses_foreign_folders() {
        let (tmp, dirs) = setup();
        let bogus = Candidate {
            source: Source::Omsi2,
            root: "Sceneryobjects".into(),
            relative: "Texture".into(),
            path: tmp.path().join("OMSI 2/Texture"),
            file_count: 0,
            size_bytes: 0,
        };
        let out = apply(&dirs, &[bogus.clone()], &Action::MoveTo(tmp.path().join("q")), &mut |_| {});
        assert_eq!(out.failed.len(), 1);
        assert!(bogus.path.exists());
    }

    #[test]
    fn mention_parsing() {
        let m = mentioned_folders("a Vehicles/MAN/x.ovh b\r\nSplines\\Foo\\a.sli no\\Vehicles\\only");
        assert_eq!(m, vec![("splines".into(), "foo".into()), ("vehicles".into(), "man".into())]);
    }
}
