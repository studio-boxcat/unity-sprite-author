// .meta file I/O. Reads the `guid:` field from a Unity .meta (works for both
// .png.meta and .asset.meta), renders the sprite .asset.meta template in
// two shapes that exist in the corpus, and surgically rewrites the atlas
// .png.meta's `alphaIsTransparency` line so it stays in lockstep with the
// tpsheet's `alphahandling` (see `update_alpha_is_transparency`).
//
//   - Modern186: 186 bytes, no trailing spaces. The fresh-mint shape.
//   - Legacy189: 189 bytes, trailing spaces after userData/assetBundle*.
//
// `meta::detect_shape` picks the shape off an existing file so a preserve-
// branch rewrite doesn't churn the bytes. See CLAUDE.md "GUID policy" for
// the full strategy.

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use crate::yaml::guid_hex;

/// Errors raised by the `.meta` I/O helpers — disk failures plus the
/// two parse failure modes (the `guid:` line is malformed, or the file
/// has no `guid:` line at all). Surfaced through
/// [`crate::pipeline::Error::Meta`].
#[derive(Debug)]
pub enum MetaError {
    Io(io::Error),
    InvalidGuid(String),
    NoGuidField,
    NoAlphaIsTransparencyField,
    InvalidAlphaIsTransparencyValue(String),
}

impl fmt::Display for MetaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "meta io error: {e}"),
            Self::InvalidGuid(s) => write!(f, "invalid guid hex: {s:?}"),
            Self::NoGuidField => write!(f, "meta has no `guid:` field"),
            Self::NoAlphaIsTransparencyField => {
                write!(f, "png.meta has no `alphaIsTransparency:` field")
            }
            Self::InvalidAlphaIsTransparencyValue(v) => {
                write!(f, "png.meta `alphaIsTransparency:` value not 0/1: {v:?}")
            }
        }
    }
}

impl std::error::Error for MetaError {}

/// Pull the 16-byte GUID out of a Unity `.meta` payload. Works on any
/// `.meta` (`.png.meta`, `.asset.meta`, etc.) since the `guid:` line
/// shape is shared. Returns [`MetaError::NoGuidField`] if missing,
/// [`MetaError::InvalidGuid`] if the hex is malformed.
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

/// Read a Unity `.meta` file from disk and extract the GUID via
/// [`parse_guid`].
pub fn read_guid<P: AsRef<Path>>(meta_path: P) -> Result<[u8; 16], MetaError> {
    let text = fs::read_to_string(meta_path).map_err(MetaError::Io)?;
    parse_guid(&text)
}

/// Trailing-space style of a sprite `.asset.meta`. Two emit shapes
/// coexist in the corpus and the pipeline preserves whichever the
/// existing file uses (see [`MetaShape`]). Three bytes / line differ
/// between the two — the cumulative difference is what gives the 186
/// vs 189 byte counts in the variant names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaFormat {
    /// Current Unity emit: `userData:\n` (no trailing space). 186 bytes.
    Modern186,
    /// Older Unity emit: `userData: \n` (with trailing space, plus the
    /// same on `assetBundleName` and `assetBundleVariant`). 189 bytes.
    Legacy189,
}

/// Per-file emit shape preserved across rewrites. Sprite `.asset.meta`
/// varies along two independent axes: [`MetaFormat`] (Modern186 vs
/// Legacy189 trailing-space style) and `mainObjectFileID` (usually
/// `21300000`, the Sprite class fileID, but transient / incompletely-
/// imported sprites carry `0`). The pipeline preserves both axes when an
/// existing file is present so a rewrite is byte-stable; fresh mints use
/// [`MetaShape::FRESH`] (Modern186 + `21300000`).
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

/// Detect the [`MetaFormat`] (trailing-space style) of an existing
/// `.asset.meta` payload by sniffing the `userData:` line. For
/// preserve-branch rewrites; fresh mints unconditionally use
/// [`MetaFormat::Modern186`].
pub fn detect_format(meta_text: &str) -> MetaFormat {
    if meta_text.contains("  userData: \n") {
        MetaFormat::Legacy189
    } else {
        MetaFormat::Modern186
    }
}

/// Detect the full [`MetaShape`] (format + `mainObjectFileID`) of an
/// existing `.asset.meta`. Defaults `mainObjectFileID` to `21300000`
/// (the Sprite class fileID) when the line is missing or unparseable.
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

/// Render a sprite `.asset.meta` for `guid` in the exact `shape`. The
/// canonical entry point used by the preserve branch (which threads the
/// detected shape through) — the simpler [`render_asset_meta`] wraps
/// this with [`MetaShape::FRESH`] for the mint path.
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

/// Render a sprite `.asset.meta` in the given `MetaFormat` with the
/// default `mainObjectFileID: 21300000`. Used by the legacy-format
/// byte-equality test against `Cake__DecoLeft`; the production pipeline
/// goes through [`render_asset_meta_with_shape`].
pub fn render_asset_meta_with_format(guid: &[u8; 16], format: MetaFormat) -> String {
    render_asset_meta_with_shape(
        guid,
        MetaShape {
            format,
            main_object_file_id: 21300000,
        },
    )
}

/// Render a sprite `.asset.meta` in `MetaShape::FRESH` (Modern186 +
/// `mainObjectFileID: 21300000`) — the shape every fresh mint uses.
pub fn render_asset_meta(guid: &[u8; 16]) -> String {
    render_asset_meta_with_shape(guid, MetaShape::FRESH)
}

/// Resolve `(guid, shape)` for a sprite. If the `.asset.meta` exists,
/// both are read from it (preserve branch — the same bytes go back out);
/// otherwise mint a fresh GUID with [`MetaShape::FRESH`] (the mint
/// branch). Used by [`crate::pipeline::generate`] so the inline
/// read+detect doesn't drift from the helper API.
pub fn resolve_sprite_meta<P: AsRef<Path>>(
    asset_meta_path: P,
) -> Result<([u8; 16], MetaShape), MetaError> {
    match fs::read_to_string(asset_meta_path) {
        Ok(text) => Ok((parse_guid(&text)?, detect_shape(&text))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok((mint_guid(), MetaShape::FRESH)),
        Err(e) => Err(MetaError::Io(e)),
    }
}

/// Rewrite a png `.meta` so its `alphaIsTransparency:` line reflects `want`:
/// `true` → `1` (Unity default, straight alpha), `false` → `0` (premultiplied).
/// Source of truth is the `.tpsheet` header's `alphahandling` (PremultiplyAlpha
/// / KeepTransparentPixels → `false`; anything else → `true`).
///
/// Surgical: edits only the matching line, preserves leading indent and the
/// rest of the file byte-stably so an idempotent re-run round-trips identical.
/// Errors if the field is missing ([`MetaError::NoAlphaIsTransparencyField`])
/// or carries a non-{0,1} value ([`MetaError::InvalidAlphaIsTransparencyValue`]).
pub fn update_alpha_is_transparency(
    meta_text: &str,
    want: bool,
) -> Result<String, MetaError> {
    let desired = if want { '1' } else { '0' };
    let mut out = String::with_capacity(meta_text.len());
    let mut found = false;
    for line in meta_text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("alphaIsTransparency:") {
            let value = rest.trim_end_matches(['\r', '\n']).trim();
            if value != "0" && value != "1" {
                return Err(MetaError::InvalidAlphaIsTransparencyValue(value.to_string()));
            }
            let indent_len = line.len() - trimmed.len();
            out.push_str(&line[..indent_len]);
            out.push_str("alphaIsTransparency: ");
            out.push(desired);
            out.push('\n');
            found = true;
        } else {
            out.push_str(line);
        }
    }
    if !found {
        return Err(MetaError::NoAlphaIsTransparencyField);
    }
    Ok(out)
}

/// Pull the `textureRect.{width, height}` from an existing Sprite `.asset`'s
/// `m_RD` block. The pipeline uses this to detect drift between the on-disk
/// textureRect and the rect we're about to emit; divergence surfaces as a
/// non-fatal entry in `GenerateOutput.warnings` (the on-disk bytes are
/// overwritten with the current tpsheet's rect).
///
/// Returns `None` if the file doesn't exist or the textureRect block can't be
/// parsed (no prior asset to diff against).
pub fn read_existing_texture_rect_size<P: AsRef<Path>>(asset_path: P) -> Option<(f32, f32)> {
    let text = fs::read_to_string(asset_path).ok()?;
    let mut in_block = false;
    let mut w: Option<f32> = None;
    let mut h: Option<f32> = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("textureRect:") {
            in_block = true;
            continue;
        }
        if !in_block {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("width: ") {
            w = rest.trim().parse().ok();
        } else if let Some(rest) = trimmed.strip_prefix("height: ") {
            h = rest.trim().parse().ok();
        } else if trimmed.starts_with("textureRectOffset:") || trimmed.starts_with("atlasRectOffset:") {
            // Past the rect block.
            break;
        }
        if let (Some(ww), Some(hh)) = (w, h) {
            return Some((ww, hh));
        }
    }
    None
}

/// Read `_prefix:` out of a `.tps`'s sibling `.tps.meta` ScriptedImporter
/// block. Returns `None` when the meta is missing, unreadable, the field
/// is absent (freshly-minted DefaultImporter meta), or the value is empty.
/// Shared by the CLI and the BoxcatBridge cdylib so prefix-defaulting is
/// uniform across headless packs and Unity Editor postprocess.
pub fn read_tps_prefix<P: AsRef<Path>>(tps_path: P) -> Option<String> {
    let mut meta = tps_path.as_ref().to_path_buf();
    meta.as_mut_os_string().push(".meta");
    let text = fs::read_to_string(&meta).ok()?;
    for line in text.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("_prefix:") {
            let v = rest.trim();
            if v.is_empty() {
                return None;
            }
            return Some(v.to_string());
        }
    }
    None
}

/// Extract `spritePixelsToUnits` from a `.png.meta` TextureImporter block.
/// The serialized field name is `spritePixelsToUnits` even though the C#
/// property is `spritePixelsPerUnit`.
pub fn read_png_ppu<P: AsRef<Path>>(png_path: P) -> Option<f32> {
    let mut meta = png_path.as_ref().to_path_buf();
    meta.as_mut_os_string().push(".meta");
    let text = fs::read_to_string(&meta).ok()?;
    for line in text.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("spritePixelsToUnits:") {
            return rest.trim().parse().ok();
        }
    }
    None
}

/// Compose a 128-bit GUID from two pre-derived entropy words (LE-packed:
/// `lo` → bytes 0..8, `hi` → bytes 8..16). Split out from [`mint_guid`]
/// so tests can pin the mint path against fixed entropy.
pub fn mint_guid_from(lo: u64, hi: u64) -> [u8; 16] {
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&lo.to_le_bytes());
    out[8..].copy_from_slice(&hi.to_le_bytes());
    out
}

/// Mint a random 128-bit GUID. Entropy comes from two
/// `std::collections::hash_map::RandomState` instances (each one carries
/// fresh SipHash keys seeded from the OS RNG by stdlib). Sufficient for
/// Unity GUID uniqueness; not crypto-grade.
pub fn mint_guid() -> [u8; 16] {
    use std::collections::hash_map::RandomState;
    use std::hash::BuildHasher;
    let lo = RandomState::new().hash_one(0u64);
    let hi = RandomState::new().hash_one(1u64);
    mint_guid_from(lo, hi)
}

/// Preserve the existing GUID if a sibling `.asset.meta` exists; else
/// mint fresh via [`mint_guid`]. For shape detection alongside the GUID,
/// use [`resolve_sprite_meta`] instead.
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

    // Mint branch end-to-end: a deterministic pair of entropy words produces a
    // known 16-byte GUID, which then drops into the full .asset.meta render so
    // the mint-branch output is pinned at the byte level (not just "non-zero").
    #[test]
    fn mint_guid_from_seeds_is_deterministic() {
        let g = mint_guid_from(0xDEAD_BEEF_CAFE_F00D, 0x0123_4567_89AB_CDEF);
        // LE byte order for both halves: lo[0..8] then hi[0..8].
        assert_eq!(
            g,
            [
                0x0d, 0xf0, 0xfe, 0xca, 0xef, 0xbe, 0xad, 0xde,
                0xef, 0xcd, 0xab, 0x89, 0x67, 0x45, 0x23, 0x01,
            ]
        );
        // Drop into the modern meta render so the mint branch is exercised
        // end-to-end at the byte level, not just at the helper boundary.
        let meta = render_asset_meta(&g);
        assert!(
            meta.contains("guid: 0df0fecaefbeaddeefcdab8967452301\n"),
            "rendered meta missing minted guid line: {meta}"
        );
        assert_eq!(meta.len(), 186);
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
    fn detect_format_recognizes_legacy_trailing_space() {
        // Legacy189: trailing space after `userData:` (and the other two).
        let legacy = "fileFormatVersion: 2\n\
                      guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n\
                      NativeFormatImporter:\n  \
                        externalObjects: {}\n  \
                        mainObjectFileID: 21300000\n  \
                        userData: \n  \
                        assetBundleName: \n  \
                        assetBundleVariant: \n";
        assert_eq!(detect_format(legacy), MetaFormat::Legacy189);
    }

    #[test]
    fn detect_format_recognizes_modern_no_trailing_space() {
        let modern = "fileFormatVersion: 2\n\
                      guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n\
                      NativeFormatImporter:\n  \
                        externalObjects: {}\n  \
                        mainObjectFileID: 21300000\n  \
                        userData:\n  \
                        assetBundleName:\n  \
                        assetBundleVariant:\n";
        assert_eq!(detect_format(modern), MetaFormat::Modern186);
    }

    #[test]
    fn read_existing_texture_rect_size_picks_up_dimensions() {
        // Synthesize an .asset snippet with a textureRect block that the
        // pipeline diff path would consult.
        let dir = std::env::temp_dir().join("uspa_test_read_texrect_sizes");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Foo.asset");
        let body = "Sprite:\n  \
                      m_Rect:\n    \
                        serializedVersion: 2\n    \
                        x: 0\n    y: 0\n    \
                        width: 80\n    height: 80\n  \
                      textureRect:\n    \
                        serializedVersion: 2\n    \
                        x: 5\n    y: 7\n    \
                        width: 78.5\n    height: 79.25\n  \
                      textureRectOffset: {x: 0, y: 0}\n";
        std::fs::write(&path, body).unwrap();
        let got = read_existing_texture_rect_size(&path).unwrap();
        assert_eq!(got, (78.5, 79.25));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_existing_texture_rect_size_missing_file_returns_none() {
        let p = std::env::temp_dir().join("uspa_does_not_exist_texrect.asset");
        let _ = std::fs::remove_file(&p);
        assert!(read_existing_texture_rect_size(&p).is_none());
    }

    #[test]
    fn read_existing_texture_rect_size_no_block_returns_none() {
        let dir = std::env::temp_dir().join("uspa_test_no_texrect_block");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Bar.asset");
        std::fs::write(&path, "Sprite:\n  m_Rect:\n    width: 1\n").unwrap();
        assert!(read_existing_texture_rect_size(&path).is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_tps_prefix_picks_up_value() {
        let dir = std::env::temp_dir().join("uspa_test_read_tps_prefix_value");
        std::fs::create_dir_all(&dir).unwrap();
        let tps = dir.join("Atlas.tps");
        std::fs::write(
            tps.with_extension("tps.meta"),
            "fileFormatVersion: 2\nguid: 0\nScriptedImporter:\n  _prefix: AC_\n",
        )
        .unwrap();
        assert_eq!(read_tps_prefix(&tps).as_deref(), Some("AC_"));
        let _ = std::fs::remove_file(tps.with_extension("tps.meta"));
    }

    #[test]
    fn read_tps_prefix_empty_value_returns_none() {
        let dir = std::env::temp_dir().join("uspa_test_read_tps_prefix_empty");
        std::fs::create_dir_all(&dir).unwrap();
        let tps = dir.join("Atlas.tps");
        std::fs::write(
            tps.with_extension("tps.meta"),
            "ScriptedImporter:\n  _prefix: \n",
        )
        .unwrap();
        assert!(read_tps_prefix(&tps).is_none());
        let _ = std::fs::remove_file(tps.with_extension("tps.meta"));
    }

    #[test]
    fn read_tps_prefix_missing_meta_returns_none() {
        let tps = std::env::temp_dir().join("uspa_does_not_exist_prefix.tps");
        let _ = std::fs::remove_file(tps.with_extension("tps.meta"));
        assert!(read_tps_prefix(&tps).is_none());
    }

    #[test]
    fn read_tps_prefix_no_field_returns_none() {
        let dir = std::env::temp_dir().join("uspa_test_read_tps_prefix_nofield");
        std::fs::create_dir_all(&dir).unwrap();
        let tps = dir.join("Atlas.tps");
        std::fs::write(
            tps.with_extension("tps.meta"),
            "fileFormatVersion: 2\nguid: 0\nDefaultImporter:\n  externalObjects: {}\n",
        )
        .unwrap();
        assert!(read_tps_prefix(&tps).is_none());
        let _ = std::fs::remove_file(tps.with_extension("tps.meta"));
    }

    #[test]
    fn update_alpha_flips_one_to_zero() {
        // Orgel fixture: alphaIsTransparency: 0 already matches PremultiplyAlpha.
        // Construct a 1 variant and ask for transparent=false.
        let src = ATLAS_META.replace("alphaIsTransparency: 0", "alphaIsTransparency: 1");
        let out = update_alpha_is_transparency(&src, false).unwrap();
        assert!(out.contains("alphaIsTransparency: 0\n"));
        assert!(!out.contains("alphaIsTransparency: 1"));
        // Surrounding bytes unchanged.
        let expected = src.replace("alphaIsTransparency: 1", "alphaIsTransparency: 0");
        assert_eq!(out, expected);
    }

    #[test]
    fn update_alpha_flips_zero_to_one() {
        // Orgel fixture is the 0 case; flip to 1 (transparent default).
        let out = update_alpha_is_transparency(ATLAS_META, true).unwrap();
        assert!(out.contains("alphaIsTransparency: 1\n"));
        assert!(!out.contains("alphaIsTransparency: 0\n"));
        let expected = ATLAS_META.replace("alphaIsTransparency: 0", "alphaIsTransparency: 1");
        assert_eq!(out, expected);
    }

    #[test]
    fn update_alpha_noop_when_already_matches_is_byte_identical() {
        let out = update_alpha_is_transparency(ATLAS_META, false).unwrap();
        assert_eq!(out, ATLAS_META, "matching value must round-trip byte-identical");
    }

    #[test]
    fn update_alpha_missing_field_errors() {
        let stripped: String = ATLAS_META
            .lines()
            .filter(|l| !l.trim_start().starts_with("alphaIsTransparency:"))
            .collect::<Vec<_>>()
            .join("\n");
        let err = update_alpha_is_transparency(&stripped, true).unwrap_err();
        assert!(matches!(err, MetaError::NoAlphaIsTransparencyField), "got {err:?}");
    }

    #[test]
    fn update_alpha_invalid_value_errors() {
        // Field present but value isn't 0/1 — surface as InvalidAlphaIsTransparencyValue
        // rather than silently rewriting (the .meta is malformed).
        let src = ATLAS_META.replace("alphaIsTransparency: 0", "alphaIsTransparency: 2");
        let err = update_alpha_is_transparency(&src, false).unwrap_err();
        assert!(
            matches!(err, MetaError::InvalidAlphaIsTransparencyValue(_)),
            "got {err:?}"
        );
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
