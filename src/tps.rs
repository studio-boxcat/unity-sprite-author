// Minimal TexturePacker .tps reader. Only extracts per-sprite spriteScale.
// Mirrors the parsing logic in TexturePackerUtils.cs:200-298.
//
// .tps is a TexturePacker XML/plist file. Pivot/scale settings live in
// IndividualSpriteSettingsMap entries that pair a filename key with a
// settings struct. We only care about spriteScale.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug)]
pub enum TpsError {
    Io(io::Error),
}

impl fmt::Display for TpsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "tps io error: {e}"),
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
    // Returns InvertScale for the sprite. Falls back to 1.0 if not found
    // (mirrors TPSData.GetInvertedScale + TryGetSpriteInfo at
    // TexturePackerUtils.cs:31-41).
    //
    // The fallback split on '-' handles aliases for sprites packed from
    // sub-folders: TexturePacker writes `OrgelEvent~/BG/Day_Brush.png` as
    // a filename in the .tps but emits `BG-Day_Brush` as the alias in the
    // .tpsheet. Direct lookup misses; `Tail('-')` recovers the filename.
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
    Ok(parse_str(&text))
}

pub fn parse_str(text: &str) -> TpsData {
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
                current_scale = num.parse().ok();
            }
        }
        i += 1;
    }

    TpsData { invert_scales }
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
}
