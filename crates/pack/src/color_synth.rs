//! Pre-pack synthesis of 1×1 `Color_*.png` swatches into a TexturePacker
//! source dir.
//!
//! When a sibling `.tps.fab.json` references a polygon `color` whose
//! `Color_RRGGBB[AA].png` is missing from the `.tps`'s first-listed
//! source dir, this module drops a synthesized swatch so the next
//! `texturepacker` pack picks it up. Mirrors meow-tower's
//! `CanvasSpriteAuthor.ReplaceColorTextures` (which ran inside the
//! Editor); we run it as the pre-pack step shared by the CLI one-shot and
//! the bridge's Editor watch. No-op when there's no fab.json, no polygon
//! leaves, or every required swatch already exists.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use tps_core::TpsDoc;
use unity_sprite_author::manifest;

use crate::color_png;

/// Outcome of a synth pass — paths the next pack will pick up newly.
#[derive(Debug, Default, PartialEq)]
pub struct SynthOutcome {
    pub written_paths: Vec<PathBuf>,
}

/// Inspect `<tps_path>.fab.json` (skip if absent), enumerate polygon
/// `Color_*` references, and write a 1×1 PNG for each one that's missing
/// from the `.tps`'s first source dir. Idempotent: existing files are
/// left untouched regardless of content.
pub fn synthesize_for_tps(tps_path: &Path) -> Result<SynthOutcome, SynthError> {
    let fab_path = with_extension_suffix(tps_path, ".fab.json");
    let fab_text = match fs::read_to_string(&fab_path) {
        Ok(t) => t,
        // Only NotFound is the no-op path. Permission denied, EIO, etc.
        // are real failures that should surface (per the global "never
        // silently swallow" rule).
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(SynthOutcome::default())
        }
        Err(e) => return Err(SynthError::Io { path: fab_path, source: e }),
    };
    let manifest = manifest::parse(&fab_text).map_err(|e| SynthError::Manifest {
        path: fab_path.clone(),
        message: e.to_string(),
    })?;
    let names = manifest.polygon_sprite_names();
    if names.is_empty() {
        return Ok(SynthOutcome::default());
    }

    let doc = TpsDoc::load(tps_path).map_err(|e| SynthError::Tps {
        path: tps_path.to_path_buf(),
        message: e.to_string(),
    })?;
    let file_lists = doc.list_file_lists().map_err(|e| SynthError::Tps {
        path: tps_path.to_path_buf(),
        message: e.to_string(),
    })?;
    let tps_dir = tps_path.parent().ok_or_else(|| SynthError::NoSourceDir {
        tps_path: tps_path.to_path_buf(),
    })?;
    let source_dir = resolve_source_dir(tps_dir, &file_lists).ok_or_else(|| {
        SynthError::NoSourceDir {
            tps_path: tps_path.to_path_buf(),
        }
    })?;

    let mut written = Vec::new();
    for name in &names {
        let rgba = rgba_from_polygon_sprite(name).ok_or_else(|| SynthError::BadColorName {
            name: name.clone(),
        })?;
        let target = source_dir.join(format!("{name}.png"));
        if target.exists() {
            continue;
        }
        let bytes = color_png::encode_1x1(rgba);
        fs::write(&target, &bytes).map_err(|e| SynthError::Io {
            path: target.clone(),
            source: e,
        })?;
        written.push(target);
    }
    Ok(SynthOutcome { written_paths: written })
}

/// Resolve the first listed `.tps` source entry to a concrete directory.
/// Returns `None` when no entry exists or the first one points at a file
/// rather than a directory (we only drop sibling PNGs into dirs).
fn resolve_source_dir(tps_dir: &Path, file_lists: &[String]) -> Option<PathBuf> {
    let first = file_lists.first()?;
    let candidate = tps_dir.join(first);
    if candidate.is_dir() {
        Some(candidate)
    } else {
        None
    }
}

/// Parse RGBA from a `Color_RRGGBB` or `Color_RRGGBBAA` sprite name.
/// 6-hex defaults alpha to `0xFF`. Returns `None` for unparseable
/// input — including names that don't start with `Color_`, which
/// shouldn't appear via `Manifest::polygon_sprite_names` but might via a
/// hand-crafted call.
fn rgba_from_polygon_sprite(name: &str) -> Option<[u8; 4]> {
    let hex = name.strip_prefix("Color_")?;
    match hex.len() {
        6 => Some([
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
            0xFF,
        ]),
        8 => Some([
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
            u8::from_str_radix(&hex[6..8], 16).ok()?,
        ]),
        _ => None,
    }
}

/// `path` + `suffix` (concatenated to OsStr). Used for `<x>.tps.fab.json`
/// where simple `with_extension` would replace `.tps` with `.fab.json`.
fn with_extension_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

#[derive(Debug)]
pub enum SynthError {
    Manifest { path: PathBuf, message: String },
    Tps { path: PathBuf, message: String },
    Io { path: PathBuf, source: io::Error },
    NoSourceDir { tps_path: PathBuf },
    BadColorName { name: String },
}

impl std::fmt::Display for SynthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Manifest { path, message } => {
                write!(f, "parse {}: {message}", path.display())
            }
            Self::Tps { path, message } => {
                write!(f, "parse {}: {message}", path.display())
            }
            Self::Io { path, source } => {
                write!(f, "io {}: {source}", path.display())
            }
            Self::NoSourceDir { tps_path } => write!(
                f,
                "{}: no resolvable source dir in fileLists (expected a directory \
                 sibling of the .tps; cannot synthesize color PNGs)",
                tps_path.display()
            ),
            Self::BadColorName { name } => {
                write!(f, "bad color sprite name {name:?} (expected Color_RRGGBB[AA])")
            }
        }
    }
}

impl std::error::Error for SynthError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_from_6_hex() {
        assert_eq!(
            rgba_from_polygon_sprite("Color_32264D"),
            Some([0x32, 0x26, 0x4D, 0xFF])
        );
    }

    #[test]
    fn rgba_from_8_hex() {
        assert_eq!(
            rgba_from_polygon_sprite("Color_DEADBEEF"),
            Some([0xDE, 0xAD, 0xBE, 0xEF])
        );
    }

    #[test]
    fn rgba_rejects_unknown_prefix_or_length() {
        assert!(rgba_from_polygon_sprite("Sprite_FF0000").is_none());
        assert!(rgba_from_polygon_sprite("Color_FF").is_none());
        assert!(rgba_from_polygon_sprite("Color_NOTHEX").is_none());
    }

    #[test]
    fn synth_no_fab_json_is_noop() {
        let dir = tempdir();
        let tps = dir.join("X.tps");
        fs::write(&tps, sample_tps("X")).unwrap();
        let out = synthesize_for_tps(&tps).unwrap();
        assert!(out.written_paths.is_empty());
    }

    #[test]
    fn synth_skips_existing_png_files() {
        let dir = tempdir();
        let sprites = dir.join("X~");
        fs::create_dir_all(&sprites).unwrap();
        let existing = sprites.join("Color_FF0000.png");
        fs::write(&existing, b"pre-existing bytes").unwrap();

        let tps = dir.join("X.tps");
        fs::write(&tps, sample_tps("X")).unwrap();
        let fab = dir.join("X.tps.fab.json");
        fs::write(
            &fab,
            r#"{ "version":1, "combined":[
                {"name":"A","mode":"ui","children":[
                    {"type":"polygon","color":"FF0000","vertices":[[0,0],[1,0],[1,1]]}
                ]}
            ]}"#,
        )
        .unwrap();

        let out = synthesize_for_tps(&tps).unwrap();
        assert!(out.written_paths.is_empty());
        // Existing file untouched.
        let bytes = fs::read(&existing).unwrap();
        assert_eq!(bytes, b"pre-existing bytes");
    }

    #[test]
    fn synth_writes_missing_swatch_into_source_dir() {
        let dir = tempdir();
        fs::create_dir_all(dir.join("X~")).unwrap();
        let tps = dir.join("X.tps");
        fs::write(&tps, sample_tps("X")).unwrap();
        let fab = dir.join("X.tps.fab.json");
        fs::write(
            &fab,
            r#"{ "version":1, "combined":[
                {"name":"A","mode":"ui","children":[
                    {"type":"polygon","color":"DEADBE","vertices":[[0,0],[1,0],[1,1]]}
                ]}
            ]}"#,
        )
        .unwrap();

        let out = synthesize_for_tps(&tps).unwrap();
        let expected = dir.join("X~/Color_DEADBE.png");
        assert_eq!(out.written_paths, vec![expected.clone()]);
        let bytes = fs::read(&expected).unwrap();
        // First 8 bytes are the PNG signature.
        assert_eq!(&bytes[..8], &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
    }

    #[test]
    fn synth_errors_when_no_source_dir_resolves() {
        let dir = tempdir();
        let tps = dir.join("X.tps");
        // .tps declares `X~/` but no such dir exists in tempdir.
        fs::write(&tps, sample_tps("X")).unwrap();
        let fab = dir.join("X.tps.fab.json");
        fs::write(
            &fab,
            r#"{ "version":1, "combined":[
                {"name":"A","mode":"ui","children":[
                    {"type":"polygon","color":"FFFFFF","vertices":[[0,0],[1,0],[1,1]]}
                ]}
            ]}"#,
        )
        .unwrap();

        let err = synthesize_for_tps(&tps).unwrap_err();
        assert!(matches!(err, SynthError::NoSourceDir { .. }));
    }

    /// Canonical `.tps` scaffold from `tps_core` — declares a `<name>~`
    /// source dir. Match `name` to the parent dir in tests so the
    /// resolver finds it.
    fn sample_tps(atlas_name: &str) -> String {
        tps_core::emit_template(atlas_name)
    }

    fn tempdir() -> PathBuf {
        let mut p = std::env::temp_dir();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        p.push(format!("color_synth_test_{nonce}"));
        fs::create_dir_all(&p).unwrap();
        p
    }
}
