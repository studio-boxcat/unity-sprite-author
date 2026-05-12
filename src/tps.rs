// Minimal TexturePacker .tps reader. Only extracts per-sprite spriteScale.
// Mirrors the IndividualSpriteSettingsMap walk in
// meow-tower/Packages/com.boxcat.libs/TexturePacker/TexturePackerUtils.cs
// (search for `spriteScaleKeyLine`).
//
// .tps is a TexturePacker XML/plist file. Pivot/scale settings live in
// IndividualSpriteSettingsMap entries that pair a filename key with a
// settings struct. We only care about spriteScale.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

/// Errors raised by [`parse`] / [`parse_str`]. Disk I/O failures and the
/// one parse failure mode (a `spriteScale` value that doesn't parse as
/// `f32`). Surfaced through [`crate::pipeline::Error::Tps`].
#[derive(Debug)]
pub enum TpsError {
    Io(io::Error),
    BadSpriteScale { line: usize, value: String },
}

impl fmt::Display for TpsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "tps io error: {e}"),
            Self::BadSpriteScale { line, value } => {
                write!(f, "malformed spriteScale at line {line}: {value:?}")
            }
        }
    }
}

impl std::error::Error for TpsError {}

#[derive(Debug, Clone, Default)]
pub struct TpsData {
    // Key: sprite filename without extension (e.g. "Cake__DecoLeft").
    // Value: InvertScale = 1 / spriteScale_in_tps. Matches the C# `InvertScale`
    // computed by TexturePackerUtils.Parse.
    pub invert_scales: HashMap<String, f32>,
}

impl TpsData {
    /// Return `1.0 / spriteScale` for the sprite, falling back to `1.0`
    /// when the alias isn't found (mirrors `TPSData.GetInvertedScale` +
    /// `TryGetSpriteInfo` in
    /// `meow-tower/Packages/com.boxcat.libs/TexturePacker/TexturePackerUtils.cs`).
    ///
    /// The fallback split on `'-'` handles aliases for sprites packed from
    /// sub-folders: TexturePacker writes `OrgelEvent~/BG/Day_Brush.png` as
    /// a filename in the `.tps` but emits `BG-Day_Brush` as the alias in
    /// the `.tpsheet`. Direct lookup misses; the suffix-after-last-`-`
    /// (C# side: `Tail('-')`) recovers the filename.
    pub fn invert_scale(&self, sprite_alias: &str) -> f32 {
        if let Some(s) = self.invert_scales.get(sprite_alias) {
            return *s;
        }
        if let Some(idx) = sprite_alias.rfind('-')
            && let Some(s) = self.invert_scales.get(&sprite_alias[idx + 1..])
        {
            return *s;
        }
        1.0
    }
}

const FILENAME_START: &str = "            <key type=\"filename\">";
const FILENAME_END: &str = "</key>";
const SETTINGS_START: &str = "            <struct type=\"IndividualSpriteSettings\">";
const SETTINGS_END: &str = "            </struct>";
const SPRITE_SCALE_KEY: &str = "                <key>spriteScale</key>";
const DOUBLE_START: &str = "                <double>";
const DOUBLE_END: &str = "</double>";

pub fn parse<P: AsRef<Path>>(path: P) -> Result<TpsData, TpsError> {
    let text = fs::read_to_string(path).map_err(TpsError::Io)?;
    parse_str(&text)
}

pub fn parse_str(text: &str) -> Result<TpsData, TpsError> {
    let mut invert_scales = HashMap::new();
    let mut pending_filenames: Vec<String> = Vec::new();
    let mut in_settings = false;
    let mut current_scale: Option<f32> = None;

    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some(rest) = line.strip_prefix(FILENAME_START) {
            if let Some(rel) = rest.strip_suffix(FILENAME_END) {
                let stem = filename_stem(rel);
                pending_filenames.push(stem);
            }
        } else if line.starts_with(SETTINGS_START) {
            in_settings = true;
            current_scale = None;
        } else if in_settings && line.starts_with(SETTINGS_END) {
            if let Some(tps_scale) = current_scale {
                let invert = 1.0_f32 / tps_scale;
                for name in pending_filenames.drain(..) {
                    invert_scales.insert(name, invert);
                }
            } else {
                pending_filenames.clear();
            }
            in_settings = false;
            current_scale = None;
        } else if in_settings && line == SPRITE_SCALE_KEY {
            i += 1;
            if let Some(v) = lines.get(i)
                && let Some(rest) = v.trim_start().strip_prefix(DOUBLE_START.trim_start())
                && let Some(num) = rest.strip_suffix(DOUBLE_END)
            {
                current_scale = Some(num.parse().map_err(|_| TpsError::BadSpriteScale {
                    line: i + 1,
                    value: num.to_string(),
                })?);
            }
        }
        i += 1;
    }

    Ok(TpsData { invert_scales })
}

fn filename_stem(rel_path: &str) -> String {
    // rel_path looks like "Sprites~/Cake__DecoLeft.png".
    let after_slash = match rel_path.rfind('/') {
        Some(idx) => &rel_path[idx + 1..],
        None => rel_path,
    };
    match after_slash.rfind('.') {
        Some(idx) => after_slash[..idx].to_string(),
        None => after_slash.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orgel_invert_scales() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden/orgel/Orgel.tps");
        let data = parse(&path).unwrap();
        // 62 sprites in Orgel.
        assert_eq!(data.invert_scales.len(), 62);
        // Cake__DecoLeft has tps spriteScale 0.8 in current .tps.
        // (Note the .asset goldens were emitted with spriteScale=1.0; this is
        // the .tps drift documented in TODO.md.)
        let s = data.invert_scale("Cake__DecoLeft");
        assert!((s - (1.0 / 0.8)).abs() < 1e-6, "got {s}");
    }

    #[test]
    fn unknown_sprite_falls_back_to_one() {
        let data = TpsData::default();
        assert_eq!(data.invert_scale("nonexistent"), 1.0);
    }

    #[test]
    fn malformed_sprite_scale_errors() {
        let bogus = r#"<key type="filename">x.png</key>
            <struct type="IndividualSpriteSettings">
                <key>spriteScale</key>
                <double>not-a-number</double>
            </struct>"#;
        match parse_str(bogus).unwrap_err() {
            TpsError::BadSpriteScale { value, .. } => assert!(value.contains("not-a-number")),
            e => panic!("expected BadSpriteScale, got {e:?}"),
        }
    }
}
