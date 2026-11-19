//! Compiles the Slint UI and generates the translations from `lang/strings.txt`.
//!
//! `lang/strings.txt` has one line per string: `English|Español|Deutsch|Polski|Čeština|Français`.
//! It produces gettext `.po` files for Slint's bundled translations (used by `@tr(...)`) and a
//! lookup table for the strings that are built in Rust code (`src/i18n.rs`).

use std::path::PathBuf;
use std::{env, fs};

const CODES: [&str; 6] = ["en", "es", "de", "pl", "cs", "fr"];

fn main() {
    println!("cargo:rerun-if-changed=lang/strings.txt");
    println!("cargo:rerun-if-changed=ui");

    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let text = fs::read_to_string("lang/strings.txt").expect("lang/strings.txt");
    let rows: Vec<Vec<&str>> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.split('|').collect())
        .collect();
    for r in &rows {
        assert_eq!(r.len(), CODES.len(), "expected {} columns in {:?}", CODES.len(), r);
    }

    for (i, code) in CODES.iter().enumerate() {
        let mut po = format!(
            "msgid \"\"\nmsgstr \"\"\n\"Content-Type: text/plain; charset=UTF-8\\n\"\n\"Language: {code}\\n\"\n\n"
        );
        for r in &rows {
            po.push_str(&format!("msgid \"{}\"\nmsgstr \"{}\"\n\n", r[0], r[i]));
        }
        let dir = out.join("lang").join(code).join("LC_MESSAGES");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("app.po"), po).unwrap();
    }

    let mut table = String::from("pub static TABLE: &[[&str; 6]] = &[\n");
    for r in &rows {
        table.push_str(&format!("    {:?},\n", r));
    }
    table.push_str("];\n");
    fs::write(out.join("i18n_table.rs"), table).unwrap();

    let config = slint_build::CompilerConfiguration::new().with_bundled_translations(out.join("lang"));
    slint_build::compile_with_config("ui/main.slint", config).expect("Slint compilation failed");

    // Windows: embed the application icon into the executable.
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=../assets/logo.ico");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../assets/logo.ico");
        res.set("ProductName", "OMSI-Tools-NG");
        res.set("FileDescription", "OMSI-Tools-NG");
        res.compile().expect("could not embed the icon");
    }
}
