# Contributing

Thanks for helping! A few ground rules:

- **Scanning is read-only.** Code that analyses an OMSI install must never write to it. Anything that
  changes files (the cleanup) must preview first, ask for confirmation, and move or recycle –
  never hard-delete.
- **A referenced file is present if it exists in any configured directory** (OMSI 2, neoOMSI,
  openOMSI). Keep that rule in the scanner and the cleanup.
- Keep logic in `core` (no UI) and cover it with unit tests that use small fixture trees
  (see `core/src/scanner.rs`).
- UI strings: use `@tr("" => "...")` in Slint or `i18n::t` / `i18n::tf` in Rust, and add the string
  in all six languages to `app/lang/strings.txt`.
- Commit titles follow conventional commits (`feat: ...`, `fix: ...`).
- By contributing you agree that your work is released under the GPL v2 (see `LICENSE`).

```
cargo fmt
cargo test
cargo run -p app
```
