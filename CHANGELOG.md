# Changelog

## 1.0.0-beta.1

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
