# Changelog

## 1.1.0

### New
- Linux build (x86_64), built and published by GitHub Actions.
- Cleanup never reports the `nEditor`, `nEditorGenerated` and `nEditorThumbnails` folders of Sceneryobjects and Splines.
- Cleanup also follows relative references (e.g. `..Scriptsdoor.osc`) between folders, so shared script folders are not reported as unused.

## 1.1.0-beta.2

### New
- Cleanup never reports the `nEditor`, `nEditorGenerated` and `nEditorThumbnails` folders of Sceneryobjects and Splines.
- Cleanup also follows relative references (e.g. `..Scriptsdoor.osc`) between folders, so shared script folders are not reported as unused.

## 1.1.0-beta.1

### New
- Linux build (x86_64): the same map verification and cleanup, built and published by GitHub Actions.
- Continuous integration: tests and a release build run on every push.

## 1.0.0

### New
- Rust rewrite of the map verification and cleanup tools of OMSI-Tools (workspace with a UI-free
  `core` crate and a Slint `app`).
- Support for OMSI 2, neoOMSI and openOMSI directories; a file referenced by a map counts as present
  if it exists in any configured directory.
- Map verification with an overview table (total / missing), per-type tabs and the map picture.
- Cleanup with separate Sceneryobjects, Splines and Vehicles lists; preview first, then move to a
  folder or the recycle bin (never hard-delete).
- Six UI languages (English, Español, Deutsch, Polski, Čeština, Français), light and dark themes.
- Read-only command line examples `scan` and `cleanup_preview`.
- Logo, Windows application icon (`assets/logo.ico`), project website (`docs/`) and documentation.
- Fast: a map with about 7,500 referenced files is verified in about 2 seconds; analysing 51 maps for cleanup takes about 20 seconds.
