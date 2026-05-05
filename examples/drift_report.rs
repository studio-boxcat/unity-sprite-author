// One-shot drift diagnostic: per atlas, sample one mismatched sprite, show
// the line-level diff between Unity-emitted golden and our emit so it's
// obvious whether the divergence is .tps drift (e.g. m_PixelsToUnits or
// m_Pivot changed) or a remaining formula bug.

use std::fs;
use std::path::{Path, PathBuf};

use unity_sprite_author::{
    emit::{self, SpriteAsset},
    meta,
    render_data::{self, AtlasSize},
    tps, tpsheet,
};

fn find_tpsheets(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            find_tpsheets(&p, out);
        } else if p.extension().is_some_and(|e| e == "tpsheet") {
            out.push(p);
        }
    }
}

fn extract_ppu(meta_text: &str) -> Option<f32> {
    for line in meta_text.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("spritePixelsToUnits: ") {
            return rest.trim().parse().ok();
        }
    }
    None
}

fn extract_prefix(p: &Path) -> String {
    let Ok(text) = fs::read_to_string(p) else {
        return String::new();
    };
    for line in text.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("_prefix: ") {
            return rest.trim_end().to_string();
        }
    }
    String::new()
}

fn is_legacy(meta_text: &str) -> bool {
    let mut tt: Option<i32> = None;
    let mut sm: Option<i32> = None;
    for line in meta_text.lines() {
        let t = line.trim_start();
        if let Some(r) = t.strip_prefix("textureType: ") {
            tt = r.trim().parse().ok();
        } else if let Some(r) = t.strip_prefix("spriteMode: ") {
            sm = r.trim().parse().ok();
        }
    }
    tt == Some(8) && sm == Some(2)
}

// Return the first 3 lines that differ (left = golden, right = ours).
fn line_diff(golden: &str, ours: &str, max: usize) -> Vec<(usize, String, String)> {
    let g: Vec<&str> = golden.lines().collect();
    let o: Vec<&str> = ours.lines().collect();
    let mut diffs = Vec::new();
    for i in 0..g.len().max(o.len()) {
        let gl = g.get(i).copied().unwrap_or("<EOF>");
        let ol = o.get(i).copied().unwrap_or("<EOF>");
        if gl != ol {
            diffs.push((i + 1, gl.to_string(), ol.to_string()));
            if diffs.len() >= max {
                break;
            }
        }
    }
    diffs
}

fn main() {
    let root = Path::new("/Users/jameskim/Develop/meow-tower/Assets");
    let mut tpsheets = Vec::new();
    find_tpsheets(root, &mut tpsheets);
    tpsheets.sort();

    let mut shown_atlases = std::collections::HashSet::new();

    for tpsheet_path in &tpsheets {
        let parent = tpsheet_path.parent().unwrap();
        let base = tpsheet_path.file_stem().unwrap().to_string_lossy().to_string();
        let png_meta_path = parent.join(format!("{base}.png.meta"));
        let tps_path = parent.join(format!("{base}.tps"));
        let tpsheet_meta_path = parent.join(format!("{base}.tpsheet.meta"));
        let sprite_dir = parent.join(&base);

        let Ok(png_meta_text) = fs::read_to_string(&png_meta_path) else {
            continue;
        };
        if is_legacy(&png_meta_text) {
            continue;
        }
        let Some(ppu) = extract_ppu(&png_meta_text) else {
            continue;
        };
        let Ok(atlas_guid) = meta::parse_guid(&png_meta_text) else {
            continue;
        };
        let prefix = extract_prefix(&tpsheet_meta_path);

        let Ok(tpsheet_text) = fs::read_to_string(tpsheet_path) else {
            continue;
        };
        let Ok(sheet) = tpsheet::parse(&tpsheet_text) else {
            continue;
        };
        let Ok(tps_data) = tps::parse(&tps_path) else {
            continue;
        };
        let atlas_size = AtlasSize {
            width: sheet.tex.width,
            height: sheet.tex.height,
        };

        for sprite in &sheet.sprites {
            let asset_name = format!("{prefix}{}", sprite.name);
            let asset_path = sprite_dir.join(format!("{asset_name}.asset"));
            let meta_path = sprite_dir.join(format!("{asset_name}.asset.meta"));
            let Ok(golden) = fs::read_to_string(&asset_path) else {
                continue;
            };
            let Ok(meta_text) = fs::read_to_string(&meta_path) else {
                continue;
            };
            let Ok(own_guid) = meta::parse_guid(&meta_text) else {
                continue;
            };

            let invert = tps_data.invert_scale(&sprite.name);
            let ptu = ppu / invert;
            let rd = render_data::build(
                sprite.rect,
                sprite.pivot,
                &sprite.geometry.vertices,
                &sprite.geometry.triangles,
                ppu,
                invert,
                atlas_size,
            );
            let asset = SpriteAsset {
                name: asset_name.clone(),
                rect: sprite.rect,
                border: sprite.border,
                pivot: sprite.pivot,
                pixels_to_units: ptu,
                own_guid,
                atlas_guid,
                render_data: rd,
                texture_rect_size: None,
            };
            // emit::emit is currently infallible (EmitError is uninhabited),
            // but go through the Result so this stays valid if a hard-fail
            // condition is added later.
            let ours = match emit::emit(&asset) {
                Ok(s) => s,
                Err(e) => match e {},
            };
            if ours == golden {
                continue;
            }

            let atlas_key = parent
                .strip_prefix(root)
                .map(|p| p.join(&base).to_string_lossy().into_owned())
                .unwrap_or_else(|_| base.clone());
            if !shown_atlases.insert(atlas_key.clone()) {
                continue; // one example per atlas
            }

            let diffs = line_diff(&golden, &ours, 5);
            println!("\n=== {} :: {asset_name} ===", atlas_key);
            println!("    invert_scale={invert}  ppu={ppu}  ptu={ptu}");
            for (line_no, g, o) in &diffs {
                println!("  L{line_no}");
                println!("    golden: {g}");
                println!("    ours:   {o}");
            }
        }
    }
}
