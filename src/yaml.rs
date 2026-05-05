// Unity-flavor formatting primitives. Centralized so divergence from C#
// `ToString("R")` is fixable in one place when a future corpus value
// breaks the assumption.

use std::fmt::Write;

// Format a 16-byte GUID as 32 lowercase hex characters (Unity convention).
pub fn guid_hex(guid: &[u8; 16]) -> String {
    let mut s = String::with_capacity(32);
    for b in guid {
        write!(&mut s, "{b:02x}").unwrap();
    }
    s
}

// Unity emits floats via C# ToString("R") which is shortest-roundtrip with no
// trailing `.0` for integer-valued floats. Rust's f32 Display matches this on
// every value in our golden corpus (probed empirically: 80, 0.5, 567.5,
// 0.4920635, -0.025804598, etc.). Diverging fixtures will surface in golden
// tests; treat as a TODO when one appears.
pub fn float(v: f32) -> String {
    format!("{v}")
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
}
