//! Case-insensitive resolving of OMSI's Windows-style relative paths.
//!
//! OMSI files reference content with backslashes and arbitrary capitalisation
//! (`Sceneryobjects\Foo\bar.sco`). Windows resolves that on its own; on other file systems the
//! path is walked segment by segment against the real directory listing.

use std::fs;
use std::path::{Path, PathBuf};

/// Splits an OMSI relative path into its segments. Returns `None` for paths that try to
/// leave the base directory (`..`) or are absolute.
pub fn segments(relative: &str) -> Option<Vec<&str>> {
    let rel = relative.trim();
    if rel.is_empty() || rel.contains(':') {
        return None;
    }
    let mut out = Vec::new();
    for seg in rel.split(['\\', '/']) {
        match seg {
            "" | "." => {}
            ".." => return None,
            s => out.push(s),
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Resolves `relative` below `base`, returning the real path if the target exists.
pub fn resolve_existing(base: &Path, relative: &str) -> Option<PathBuf> {
    let segs = segments(relative)?;

    // Fast path: correct spelling (always taken on case-insensitive file systems).
    let mut plain = base.to_path_buf();
    plain.extend(&segs);
    if plain.exists() {
        return Some(plain);
    }

    let mut cur = base.to_path_buf();
    for seg in segs {
        let direct = cur.join(seg);
        if direct.exists() {
            cur = direct;
            continue;
        }
        let lower = seg.to_lowercase();
        let found = fs::read_dir(&cur)
            .ok()?
            .filter_map(Result::ok)
            .find(|e| e.file_name().to_string_lossy().to_lowercase() == lower)?;
        cur = found.path();
    }
    Some(cur)
}

/// Normalised key for comparing OMSI paths: forward slashes, lower case, no leading slash.
pub fn key(relative: &str) -> String {
    relative
        .trim()
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_lowercase()
}

/// `path` relative to `base` in OMSI backslash notation (None if not inside `base`).
pub fn relative_backslash(base: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(base).ok()?;
    Some(
        rel.components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("\\"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_backslashes_and_case() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("Sceneryobjects/Foo")).unwrap();
        fs::write(tmp.path().join("Sceneryobjects/Foo/bar.sco"), "x").unwrap();

        let p = resolve_existing(tmp.path(), "sceneryobjects\\FOO\\Bar.SCO").unwrap();
        assert!(p.is_file());
        assert!(resolve_existing(tmp.path(), "Sceneryobjects\\Foo\\nope.sco").is_none());
    }

    #[test]
    fn rejects_escaping_paths() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(resolve_existing(tmp.path(), "..\\x").is_none());
        assert!(resolve_existing(tmp.path(), "C:\\Windows").is_none());
        assert!(resolve_existing(tmp.path(), "").is_none());
    }

    #[test]
    fn key_and_relative() {
        assert_eq!(key("\\Texture\\A.bmp "), "texture/a.bmp");
        let base = Path::new("/x");
        assert_eq!(relative_backslash(base, &base.join("a").join("b")).unwrap(), "a\\b");
    }
}
