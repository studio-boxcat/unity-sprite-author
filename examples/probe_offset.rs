// Throwaway probe: dump (rect, pivot, target_offset, original_pivot_from_tps)
// for sprites where the current m_Offset formula diverges. Compile-and-run
// from the repo root:
//
//   cargo run --release --example probe_offset > /tmp/offsets.tsv
//
// Then `awk` / look for a formula that fits the column.

use std::fs;
use std::path::{Path, PathBuf};

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

fn read_field(text: &str, prefix: &str) -> Option<String> {
    for line in text.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix(prefix) {
            return Some(rest.trim_end().to_string());
        }
    }
    None
}

// Parse "{x: A, y: B}" into (A, B).
fn parse_pair(text: &str) -> Option<(f32, f32)> {
    let body = text.trim().trim_start_matches('{').trim_end_matches('}');
    let mut x = None;
    let mut y = None;
    for part in body.split(',') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix("x: ") {
            x = rest.trim().parse().ok();
        } else if let Some(rest) = part.strip_prefix("y: ") {
            y = rest.trim().parse().ok();
        }
    }
    x.zip(y)
}

// Find the original .tps pivotPoint for a sprite by name.
fn original_pivot_from_tps(tps_text: &str, sprite_name: &str) -> Option<(f32, f32)> {
    // .tps has filename keys like Sprites~/<name>.png followed by struct with pivotPoint.
    let lines: Vec<&str> = tps_text.lines().collect();
    let needle = format!("{sprite_name}.png");
    let mut active = false;
    for i in 0..lines.len() {
        let line = lines[i];
        if line.contains(&needle) && line.contains("<key type=\"filename\">") {
            active = true;
            continue;
        }
        if active && line.trim() == "</struct>" {
            return None;
        }
        if active && line.trim() == "<key>pivotPoint</key>" && i + 1 < lines.len() {
            let val = lines[i + 1].trim();
            // <point_f>0.5,0.5</point_f>
            let body = val
                .trim_start_matches("<point_f>")
                .trim_end_matches("</point_f>");
            let mut parts = body.split(',');
            let x: f32 = parts.next()?.trim().parse().ok()?;
            let y: f32 = parts.next()?.trim().parse().ok()?;
            return Some((x, y));
        }
    }
    None
}

fn parse_rect_block(asset_text: &str) -> Option<(u32, u32, u32, u32)> {
    // m_Rect:
    //   serializedVersion: 2
    //   x: 1
    //   y: 179
    //   width: 103
    //   height: 92
    let mut in_rect = false;
    let mut x = None;
    let mut y = None;
    let mut w = None;
    let mut h = None;
    for line in asset_text.lines() {
        if line.trim_end() == "  m_Rect:" {
            in_rect = true;
            continue;
        }
        if !in_rect {
            continue;
        }
        let t = line.trim_start();
        if let Some(v) = t.strip_prefix("x: ") {
            x = v.trim().parse().ok();
        } else if let Some(v) = t.strip_prefix("y: ") {
            y = v.trim().parse().ok();
        } else if let Some(v) = t.strip_prefix("width: ") {
            w = v.trim().parse().ok();
        } else if let Some(v) = t.strip_prefix("height: ") {
            h = v.trim().parse().ok();
        } else if t.starts_with("m_Offset:") || t.starts_with("m_Border:") {
            break;
        }
    }
    Some((x?, y?, w?, h?))
}

fn main() {
    let root = Path::new("/Users/jameskim/Develop/meow-tower/Assets");
    let mut tpsheets = Vec::new();
    find_tpsheets(root, &mut tpsheets);
    tpsheets.sort();

    // CSV-ish header
    println!(
        "atlas\tsprite\tw\th\tpx\tpy\torig_px\torig_py\toff_x_target\toff_y_target\toff_x_bits\toff_y_bits"
    );

    let mut printed = 0usize;
    for tpsheet_path in &tpsheets {
        let parent = tpsheet_path.parent().unwrap();
        let base = tpsheet_path.file_stem().unwrap().to_string_lossy().to_string();
        let tps_path = parent.join(format!("{base}.tps"));
        let sprite_dir = parent.join(&base);
        let Ok(tps_text) = fs::read_to_string(&tps_path) else {
            continue;
        };
        let Ok(entries) = fs::read_dir(&sprite_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if !p.extension().is_some_and(|e| e == "asset") {
                continue;
            }
            let Ok(asset_text) = fs::read_to_string(&p) else {
                continue;
            };
            // Skip sprites with all-zero m_Offset to focus on the non-trivial ones.
            let off = read_field(&asset_text, "m_Offset: ").unwrap_or_default();
            if off == "{x: 0, y: 0}" {
                continue;
            }
            let Some((ox, oy)) = parse_pair(&off) else {
                continue;
            };
            let pivot = read_field(&asset_text, "m_Pivot: ").unwrap_or_default();
            let Some((px, py)) = parse_pair(&pivot) else {
                continue;
            };
            let Some((_, _, w, h)) = parse_rect_block(&asset_text) else {
                continue;
            };

            // The asset name has the prefix; strip via the tpsheet's _prefix
            // approximation. Easiest: take the file stem and try the tps lookup
            // both with and without common prefix matches. For probe purposes,
            // try the suffix after the last "_" of the prefix (often `XX_NN_`).
            let asset_stem = p.file_stem().unwrap().to_string_lossy().to_string();
            let original = original_pivot_from_tps(&tps_text, &asset_stem)
                .or_else(|| {
                    // Try stripping leading "{prefix}" by searching for any "_" delimiter.
                    asset_stem
                        .find('_')
                        .map(|i| asset_stem[i + 1..].to_string())
                        .and_then(|n| original_pivot_from_tps(&tps_text, &n))
                });

            let atlas_label = parent
                .strip_prefix(root)
                .map(|r| r.join(&base).to_string_lossy().into_owned())
                .unwrap_or_else(|_| base.clone());

            let (orig_px, orig_py) = original.unwrap_or((f32::NAN, f32::NAN));
            println!(
                "{atlas_label}\t{asset_stem}\t{w}\t{h}\t{px}\t{py}\t{orig_px}\t{orig_py}\t{ox}\t{oy}\t0x{:08x}\t0x{:08x}",
                ox.to_bits(),
                oy.to_bits(),
            );
            printed += 1;
            if printed >= 80 {
                return;
            }
        }
    }
}
