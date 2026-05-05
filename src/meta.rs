// .meta file I/O. Reads the `guid:` field from a Unity .meta (works for both
// .png.meta and .asset.meta), and renders the canonical 189-byte sprite
// .asset.meta template.
//
// The canonical template is verified across the meow-tower corpus
// (3645 sprite .asset.meta files, all 189 bytes, all schema-identical).
// See CLAUDE.md "GUID policy" for details.

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use crate::yaml::guid_hex;

#[derive(Debug)]
pub enum MetaError {
    Io(io::Error),
    InvalidGuid(String),
    NoGuidField,
}

impl fmt::Display for MetaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "meta io error: {e}"),
            Self::InvalidGuid(s) => write!(f, "invalid guid hex: {s:?}"),
            Self::NoGuidField => write!(f, "meta has no `guid:` field"),
        }
    }
}

impl std::error::Error for MetaError {}

pub fn parse_guid(meta_text: &str) -> Result<[u8; 16], MetaError> {
    for line in meta_text.lines() {
        if let Some(rest) = line.strip_prefix("guid: ") {
            return parse_guid_hex(rest.trim());
        }
    }
    Err(MetaError::NoGuidField)
}

fn parse_guid_hex(hex: &str) -> Result<[u8; 16], MetaError> {
    if hex.len() != 32 {
        return Err(MetaError::InvalidGuid(hex.to_string()));
    }
    let mut out = [0u8; 16];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .map_err(|_| MetaError::InvalidGuid(hex.to_string()))?;
    }
    Ok(out)
}

pub fn read_guid<P: AsRef<Path>>(meta_path: P) -> Result<[u8; 16], MetaError> {
    let text = fs::read_to_string(meta_path).map_err(MetaError::Io)?;
    parse_guid(&text)
}

// Sprite .asset.meta exists in two trailing-space variants in the wild:
// the older 189-byte format (with spaces after `userData:`, `assetBundleName:`,
// `assetBundleVariant:`) and the newer 186-byte format that current Unity
// emits. To avoid byte churn on existing metas, we preserve the on-disk
// format when present and only use Modern186 for fresh mints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaFormat {
    Modern186,
    Legacy189,
}

// Detect the trailing-space format of an existing meta file. Looks for
// `userData: \n` (legacy) vs `userData:\n` (modern); returns Modern186 by
// default for anything unrecognized.
pub fn detect_format(meta_text: &str) -> MetaFormat {
    if meta_text.contains("  userData: \n") {
        MetaFormat::Legacy189
    } else {
        MetaFormat::Modern186
    }
}

// Render the canonical sprite .asset.meta in the requested format.
pub fn render_asset_meta_with_format(guid: &[u8; 16], format: MetaFormat) -> String {
    let trail = match format {
        MetaFormat::Modern186 => "",
        MetaFormat::Legacy189 => " ",
    };
    let mut s = String::with_capacity(189);
    s.push_str("fileFormatVersion: 2\n");
    s.push_str("guid: ");
    s.push_str(&guid_hex(guid));
    s.push('\n');
    s.push_str("NativeFormatImporter:\n");
    s.push_str("  externalObjects: {}\n");
    s.push_str("  mainObjectFileID: 21300000\n");
    s.push_str("  userData:");
    s.push_str(trail);
    s.push('\n');
    s.push_str("  assetBundleName:");
    s.push_str(trail);
    s.push('\n');
    s.push_str("  assetBundleVariant:");
    s.push_str(trail);
    s.push('\n');
    debug_assert_eq!(
        s.len(),
        match format { MetaFormat::Modern186 => 186, MetaFormat::Legacy189 => 189 }
    );
    s
}

// Convenience: render in the modern format (used for fresh mints).
pub fn render_asset_meta(guid: &[u8; 16]) -> String {
    render_asset_meta_with_format(guid, MetaFormat::Modern186)
}

// Mint a random 128-bit GUID. Uses two `RandomState` instances for entropy
// (each one carries fresh SipHash keys seeded from the OS RNG by stdlib).
// Sufficient for Unity GUID uniqueness; not crypto-grade.
pub fn mint_guid() -> [u8; 16] {
    use std::collections::hash_map::RandomState;
    use std::hash::BuildHasher;
    let lo = RandomState::new().hash_one(0u64);
    let hi = RandomState::new().hash_one(1u64);
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&lo.to_le_bytes());
    out[8..].copy_from_slice(&hi.to_le_bytes());
    out
}

// Preserve existing GUID if a sibling .asset.meta exists; else mint fresh.
pub fn resolve_sprite_guid<P: AsRef<Path>>(asset_meta_path: P) -> Result<[u8; 16], MetaError> {
    match fs::read_to_string(asset_meta_path) {
        Ok(text) => parse_guid(&text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(mint_guid()),
        Err(e) => Err(MetaError::Io(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAKE_DECOLEFT_META: &str =
        include_str!("../tests/golden/orgel/sprites/Cake__DecoLeft.asset.meta");
    const ATLAS_META: &str = include_str!("../tests/golden/orgel/Orgel.png.meta");

    #[test]
    fn render_emits_186_bytes_and_round_trips_guid() {
        // Cake__DecoLeft was emitted by an older Unity with trailing
        // spaces (189 bytes). Current Unity emits 186 bytes (no trailing
        // spaces). We target the current format; tested below by parsing
        // the rendered meta back and round-tripping the GUID.
        let guid = parse_guid(CAKE_DECOLEFT_META).unwrap();
        let rendered = render_asset_meta(&guid);
        assert_eq!(rendered.len(), 186, "current Unity emits 186-byte metas");
        let round_trip = parse_guid(&rendered).unwrap();
        assert_eq!(round_trip, guid);
        // Schema invariants pinned regardless of trailing-space variant.
        assert!(rendered.starts_with("fileFormatVersion: 2\n"));
        assert!(rendered.contains("mainObjectFileID: 21300000\n"));
        assert!(rendered.ends_with("assetBundleVariant:\n"));
    }

    #[test]
    fn parse_atlas_png_meta_guid() {
        let g = parse_guid(ATLAS_META).unwrap();
        // Spot-check first/last bytes against the golden's
        // m_RD.texture.guid: 65583bd2af0024cd586c22cdc38c4672
        assert_eq!(g[0], 0x65);
        assert_eq!(g[15], 0x72);
    }

    #[test]
    fn mint_guid_is_16_bytes_and_random() {
        let a = mint_guid();
        let b = mint_guid();
        // Astronomically unlikely to collide; if this fires, panic loudly.
        assert_ne!(a, b);
    }

    #[test]
    fn resolve_existing_preserves_guid() {
        let dir = std::env::temp_dir().join("uspa_test_resolve_existing");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("Foo.asset.meta");
        std::fs::write(&path, CAKE_DECOLEFT_META).unwrap();
        let g = resolve_sprite_guid(&path).unwrap();
        assert_eq!(guid_hex(&g), "d4c782eb3340c41848b2a0a903c0fcea");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn resolve_missing_mints_fresh() {
        let path = std::env::temp_dir().join("uspa_does_not_exist.asset.meta");
        let _ = std::fs::remove_file(&path); // ensure absent
        let g = resolve_sprite_guid(&path).unwrap();
        // Guid is 16 zeroes only with vanishing probability; just check it's not all-zero.
        assert_ne!(g, [0u8; 16]);
    }
}
