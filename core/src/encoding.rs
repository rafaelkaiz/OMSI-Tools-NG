//! Reading OMSI's legacy text files.
//!
//! OMSI writes ANSI (Windows-1252 on the western installs the game is distributed for).
//! Some files, e.g. `global.cfg` saved by OMSI's own editor, are UTF-16 with a byte order
//! mark, so a BOM always wins over the Windows-1252 fallback. Using a fixed codec instead of
//! the system code page keeps the results identical on every platform.

use std::io;
use std::path::Path;

use encoding_rs::{Encoding, UTF_16BE, UTF_16LE, UTF_8, WINDOWS_1252};

/// Decodes the bytes of an OMSI text file.
pub fn decode_omsi(bytes: &[u8]) -> String {
    let (enc, skip): (&'static Encoding, usize) = if bytes.starts_with(&[0xFF, 0xFE]) {
        (UTF_16LE, 2)
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        (UTF_16BE, 2)
    } else if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        (UTF_8, 3)
    } else {
        (WINDOWS_1252, 0)
    };
    let (text, _) = enc.decode_without_bom_handling(&bytes[skip..]);
    text.into_owned()
}

/// Reads and decodes a file.
pub fn read_text(path: &Path) -> io::Result<String> {
    Ok(decode_omsi(&std::fs::read(path)?))
}

/// Splits into lines (`\n` / `\r\n`), trimming trailing whitespace like OMSI does.
pub fn lines(text: &str) -> Vec<&str> {
    text.lines().map(|l| l.trim_end()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_1252_umlauts() {
        // "Hbf Münster" with ü as 0xFC
        assert_eq!(decode_omsi(b"Hbf M\xFCnster"), "Hbf Münster");
    }

    #[test]
    fn utf16le_bom() {
        let mut bytes = vec![0xFF, 0xFE];
        for u in "Köln".encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(decode_omsi(&bytes), "Köln");
    }

    #[test]
    fn utf8_bom_and_crlf() {
        let t = decode_omsi(b"\xEF\xBB\xBF[map]\r\nx \r\n");
        assert_eq!(lines(&t), vec!["[map]", "x"]);
    }
}
