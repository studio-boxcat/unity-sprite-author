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

// Sprite .asset.meta varies along two independent axes:
//   1. Trailing-space style: legacy emits `userData: \n` (with space);
//      modern emits `userData:\n` (without). 3 bytes difference per line.
//   2. mainObjectFileID: usually 21300000 (the Sprite class fileID), but
//      transient/incompletely-imported sprites carry 0 instead.
// To avoid churn on existing metas we preserve both axes when present.
// Fresh mints use Modern186 + 21300000.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaFormat {
    Modern186,
    Legacy189,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetaShape {
    pub format: MetaFormat,
    pub main_object_file_id: i64,
}

impl MetaShape {
    pub const FRESH: Self = Self {
        format: MetaFormat::Modern186,
        main_object_file_id: 21300000,
    };
}

pub fn detect_format(meta_text: &str) -> MetaFormat {
    if meta_text.contains("  userData: \n") {
        MetaFormat::Legacy189
    } else {
        MetaFormat::Modern186
    }
}

pub fn detect_shape(meta_text: &str) -> MetaShape {
    let mut id = 21300000_i64;
    for line in meta_text.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("mainObjectFileID: ")
            && let Ok(parsed) = rest.trim().parse::<i64>()
        {
            id = parsed;
            break;
        }
    }
    MetaShape {
        format: detect_format(meta_text),
        main_object_file_id: id,
    }
}

pub fn render_asset_meta_with_shape(guid: &[u8; 16], shape: MetaShape) -> String {
    let trail = match shape.format {
        MetaFormat::Modern186 => "",
        MetaFormat::Legacy189 => " ",
    };
    let mut s = String::with_capacity(192);
    s.push_str("fileFormatVersion: 2\n");
    s.push_str("guid: ");
    s.push_str(&guid_hex(guid));
    s.push('\n');
    s.push_str("NativeFormatImporter:\n");
    s.push_str("  externalObjects: {}\n");
    use std::fmt::Write;
    writeln!(s, "  mainObjectFileID: {}", shape.main_object_file_id).unwrap();
    s.push_str("  userData:");
    s.push_str(trail);
    s.push('\n');
    s.push_str("  assetBundleName:");
    s.push_str(trail);
    s.push('\n');
    s.push_str("  assetBundleVariant:");
    s.push_str(trail);
    s.push('\n');
    s
}

// Render in a specific format with the default mainObjectFileID. Used by
// the legacy-format byte-equality test against Cake__DecoLeft; production
// pipeline goes through render_asset_meta_with_shape.
pub fn render_asset_meta_with_format(guid: &[u8; 16], format: MetaFormat) -> String {
    render_asset_meta_with_shape(
        guid,
        MetaShape {
            format,
            main_object_file_id: 21300000,
        },
    )
}

pub fn render_asset_meta(guid: &[u8; 16]) -> String {
    render_asset_meta_with_shape(guid, MetaShape::FRESH)
}

// Resolve `(guid, shape)` for a sprite. If the .asset.meta exists, both
// are read from it; otherwise mint a fresh GUID with `MetaShape::FRESH`.
// Used by the pipeline so the inline read+detect doesn't drift from the
// helper API.
pub fn resolve_sprite_meta<P: AsRef<Path>>(
    asset_meta_path: P,
) -> Result<([u8; 16], MetaShape), MetaError> {
    match fs::read_to_string(asset_meta_path) {
        Ok(text) => Ok((parse_guid(&text)?, detect_shape(&text))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok((mint_guid(), MetaShape::FRESH)),
        Err(e) => Err(MetaError::Io(e)),
    }
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
    fn render_legacy189_byte_exact_against_cake_decoleft() {
        // Cake__DecoLeft was emitted by older Unity (189-byte trailing-space
        // form). Render in the matching format and assert full byte equality.
        let shape = detect_shape(CAKE_DECOLEFT_META);
        assert_eq!(shape.format, MetaFormat::Legacy189);
        let guid = parse_guid(CAKE_DECOLEFT_META).unwrap();
        let rendered = render_asset_meta_with_shape(&guid, shape);
        assert_eq!(rendered, CAKE_DECOLEFT_META);
        assert_eq!(rendered.len(), 189);
    }

    #[test]
    fn render_modern186_size_and_round_trip() {
        // The current Unity output. We don't have a 186-byte fixture
        // bundled with the crate (Cake is 189), so check the size + round-
        // trip the parse. The full-corpus byte-exactness is covered by the
        // meow-tower e2e (which detects per-file format).
        let guid = parse_guid(CAKE_DECOLEFT_META).unwrap();
        let rendered = render_asset_meta(&guid);
        assert_eq!(rendered.len(), 186);
        assert_eq!(parse_guid(&rendered).unwrap(), guid);
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
