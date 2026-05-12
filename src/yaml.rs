// Unity-flavor formatting primitives. Centralized so divergence from C#
// `ToString("R")` is fixable in one place when a future corpus value
// breaks the assumption.

// Bulk lowercase hex encoder, used by both GUID rendering and the typeless
// data / index buffer in render_data. format!("{:02x}") via write! is ~5x
// slower per byte (measured); the LUT path matters because we encode ~152
// bytes of typelessdata + 16 bytes of GUID per sprite, four times per sprite.
const HEX_LUT: &[u8; 16] = b"0123456789abcdef";

pub fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX_LUT[(b >> 4) as usize] as char);
        out.push(HEX_LUT[(b & 0xf) as usize] as char);
    }
    out
}

// Format a 16-byte GUID as 32 lowercase hex characters (Unity convention).
pub fn guid_hex(guid: &[u8; 16]) -> String {
    hex_encode(guid)
}

// Unity emits floats via C# ToString("R") which is shortest-roundtrip with no
// trailing `.0` for integer-valued floats. Rust's f32 Display matches this on
// every value in our golden corpus (probed empirically: 80, 0.5, 567.5,
// 0.4920635, -0.025804598, etc.). Diverging fixtures will surface in golden
// tests; treat as a TODO when one appears.
pub fn float(v: f32) -> String {
    format!("{v}")
}

// Unity's YAML emitter quotes strings containing non-ASCII characters and
// escapes the non-ASCII codepoints as \uXXXX (UTF-16). Codepoints above
// U+FFFF use surrogate pairs. ASCII-only strings are emitted unquoted.
// Verified against `m_Name: "OG_0503_Signboard__티켓"` (Korean
// "티켓" = U+D2F0 U+CF13).
pub fn yaml_string(s: &str) -> String {
    if s.bytes().all(|b| b.is_ascii() && b != b'"' && b != b'\\') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        let cp = c as u32;
        if cp < 0x80 {
            out.push(c);
        } else if cp <= 0xFFFF {
            use std::fmt::Write;
            write!(&mut out, "\\u{:04X}", cp).unwrap();
        } else {
            // Surrogate pair
            let v = cp - 0x10000;
            let high = 0xD800 + (v >> 10);
            let low = 0xDC00 + (v & 0x3FF);
            use std::fmt::Write;
            write!(&mut out, "\\u{:04X}\\u{:04X}", high, low).unwrap();
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guid_lowercase_no_dashes() {
        let g = [
            0xd4, 0xc7, 0x82, 0xeb, 0x33, 0x40, 0xc4, 0x18, 0x48, 0xb2, 0xa0, 0xa9, 0x03, 0xc0,
            0xfc, 0xea,
        ];
        assert_eq!(guid_hex(&g), "d4c782eb3340c41848b2a0a903c0fcea");
    }

    #[test]
    fn float_corpus_samples() {
        assert_eq!(float(80.0), "80");
        assert_eq!(float(0.5), "0.5");
        assert_eq!(float(567.5), "567.5");
        assert_eq!(float(221.0), "221");
    }

    // Seeds from every distinct fractional float literal in tests/golden/**/*.asset
    // and asserts our `float()` round-trips each one bit-exactly. Guard against
    // a future Rust Display divergence from C# `ToString("R")`.
    #[test]
    fn float_corpus_full_roundtrip() {
        use std::collections::BTreeSet;
        use std::fs;
        use std::path::Path;

        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
        let mut floats: BTreeSet<String> = BTreeSet::new();
        walk_assets(&root, &mut |path| {
            let Ok(text) = fs::read_to_string(path) else { return };
            scan_fractional_floats(&text, &mut floats);
        });

        assert!(
            floats.len() >= 90,
            "expected ≥90 distinct float literals in the golden corpus, found {}",
            floats.len()
        );

        let mut mismatches: Vec<String> = Vec::new();
        for s in &floats {
            let Ok(f) = s.parse::<f32>() else {
                mismatches.push(format!("could not parse {s:?} as f32"));
                continue;
            };
            let got = float(f);
            if &got != s {
                mismatches.push(format!(
                    "{s:?} (bits 0x{:08x}) → float() = {got:?}",
                    f.to_bits()
                ));
            }
        }
        assert!(
            mismatches.is_empty(),
            "{} float format mismatches:\n  {}",
            mismatches.len(),
            mismatches.join("\n  ")
        );
    }

    fn walk_assets(dir: &std::path::Path, f: &mut dyn FnMut(&std::path::Path)) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk_assets(&p, f);
            } else if p.extension().and_then(|e| e.to_str()) == Some("asset") {
                f(&p);
            }
        }
    }

    // Extracts Unity float-literal tokens (mandatory fractional part), boundary
    // checked so embedded substrings inside identifiers/hex never match.
    fn scan_fractional_floats(text: &str, out: &mut std::collections::BTreeSet<String>) {
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let token_start = i;
            let prev_alnum = i > 0
                && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'.');
            let mut j = i;
            if bytes[j] == b'-' {
                j += 1;
            }
            if j >= bytes.len() || !bytes[j].is_ascii_digit() {
                i = token_start + 1;
                continue;
            }
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j >= bytes.len() || bytes[j] != b'.' {
                i = j.max(token_start + 1);
                continue;
            }
            j += 1;
            if j >= bytes.len() || !bytes[j].is_ascii_digit() {
                i = token_start + 1;
                continue;
            }
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'e' {
                let exp_start = j;
                j += 1;
                if j < bytes.len() && bytes[j] == b'-' {
                    j += 1;
                }
                if j < bytes.len() && bytes[j].is_ascii_digit() {
                    while j < bytes.len() && bytes[j].is_ascii_digit() {
                        j += 1;
                    }
                } else {
                    j = exp_start;
                }
            }
            // Don't accept literals abutting a trailing alnum (would be inside
            // an identifier or hex stream).
            let next_alnum = j < bytes.len()
                && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'.');
            if !prev_alnum && !next_alnum {
                out.insert(text[token_start..j].to_string());
            }
            i = j.max(token_start + 1);
        }
    }
}
