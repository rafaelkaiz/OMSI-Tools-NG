# OMSI-Tools-NG-rs

A from-scratch **Rust rewrite** of the map scanner and cleanup features of
[OMSI-Tools](https://github.com/pietr26/OMSI-Tools) by **pietr26** (a modding tool for OMSI 2),
with a modern [Slint](https://slint.dev) UI and support for **OMSI 2**, **neoOMSI** and **openOMSI**.

## Attribution and licence

This project is derived from OMSI-Tools (C++/Qt) by pietr26 and from its fork OMSI-Tools-NG.
The file formats, parsing rules and the scan/cleanup behaviour were ported from that code base,
so the original credit is kept and the project is released under the
**GNU General Public License v2.0** (see [`LICENSE`](LICENSE)).

> Original work: Copyright (C) pietr26 and the OMSI-Tools contributors — <https://github.com/pietr26/OMSI-Tools>

## Features

- Configurable game directories: OMSI 2 (main), optional neoOMSI, optional openOMSI. Stored in a
  TOML config file.
- Map list from all directories (`maps/*/global.cfg`, `[friendlyname]`), labelled with the source.
- **Map scan** (read-only): global.cfg, tiles (incl. Chrono), ground textures, parklists, humans,
  drivers, AI lists and train files. Reports missing files grouped by type, with the files that
  reference them.
  **A referenced file counts as present if it exists in *any* configured directory**, so a
  neoOMSI map using an object that lives in OMSI 2 is valid.
- **Cleanup**: finds content folders (`Sceneryobjects`, `Splines`, `Vehicles`) that no map uses.
  It only previews by default; applying moves the folders to a quarantine folder or the
  recycle bin. Nothing is ever hard-deleted.
- OMSI's legacy encodings (Windows-1252, UTF-16/UTF-8 with BOM) and backslash paths with any
  capitalisation are handled on every platform.

## Layout

| Crate  | Purpose                                              |
|--------|------------------------------------------------------|
| `core` | UI-free library: config, parsing, scanner, cleanup   |
| `app`  | Slint desktop application                            |

## Build and run

```
cargo test
cargo run -p app
```
