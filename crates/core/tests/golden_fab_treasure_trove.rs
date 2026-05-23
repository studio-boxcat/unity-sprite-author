// End-to-end ULP-tolerance vertex/rect/pivot/offset diff for the TreasureTrove
// fab fixture, against pre-c23474b2 `.asset` goldens captured from meow-tower.
//
// Stages the committed atlas + `.tps.fab.json` into a temp dir, runs
// `pipeline::generate`, and for each of the 7 trees in this atlas reads the
// emitted `.asset`, parses its m_Rect / m_Pivot / m_Offset / _typelessdata,
// and asserts they match the golden within tolerance:
//   - rect: ±0.5 px
//   - pivot: ±0.005
//   - offset: ±0.5 px
//   - per-vertex: ±4 ULP per axis
//
// Exercises:
//   - UIIcon ID native scale
//   - UIIcon MX/MY/MXY (icon_mirror via size=None)
//   - UISlice ID stretch (slice_identity via Color_* leaves with size)
//   - L+R container mirror cascade (TT_LobbyCell_Frame, _BG)
//   - UIIcon-with-children non-default RectTransform.pivot for walker cascade

use std::fs;
use std::path::{Path, PathBuf};

use unity_sprite_author::pipeline::{self, GenerateInputs};

const SPRITES: &[&str] = &[
    "TT_LobbyCell_Frame",
    "TT_LobbyCell_BG",
    "TT_LobbyCell_Floor",
    "TT_Cabinet_Top",
];

const RECT_TOL: f32 = 0.5;
const PIVOT_TOL: f32 = 0.005;
const OFFSET_TOL: f32 = 0.5;
const ULP_TOL: i32 = 4;

#[test]
fn treasure_trove_matches_pre_migration_goldens() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/fab/treasure_trove");
    let dst: PathBuf = std::env::temp_dir().join("uspa_treasure_trove");
    let _ = fs::remove_dir_all(&dst);
    fs::create_dir_all(&dst).unwrap();

    for entry in fs::read_dir(&src).unwrap().flatten() {
        let from = entry.path();
        if from.is_file() {
            fs::copy(&from, dst.join(entry.file_name())).unwrap();
        }
    }

    let sprite_dir = dst.join("TreasureTrove");
    fs::create_dir_all(&sprite_dir).unwrap();
    for s in SPRITES {
        let meta = dst.join(format!("{s}.asset.meta"));
        if meta.exists() {
            fs::copy(&meta, sprite_dir.join(format!("{s}.asset.meta"))).unwrap();
        }
    }

    pipeline::generate(&GenerateInputs {
        tpsheet_path: &dst.join("TreasureTrove.tpsheet"),
        tps_path: &dst.join("TreasureTrove.tps"),
        atlas_png_path: &dst.join("TreasureTrove.png"),
        sprite_dir: &sprite_dir,
        prefix: "",
    })
    .expect("pipeline");

    let mut failures = Vec::<String>::new();
    for sprite in SPRITES {
        let got_path = sprite_dir.join(format!("{sprite}.asset"));
        let got = match fs::read_to_string(&got_path) {
            Ok(s) => s,
            Err(_) => { failures.push(format!("{sprite}: MISSING emitted .asset")); continue; }
        };
        let golden = fs::read_to_string(src.join(format!("{sprite}.asset"))).unwrap();
        let g = parse_asset(&golden);
        let c = parse_asset(&got);
        let mut local: Vec<String> = Vec::new();

        if let (Some(gr), Some(cr)) = (g.rect, c.rect) {
            if (gr.0 - cr.0).abs() > RECT_TOL || (gr.1 - cr.1).abs() > RECT_TOL {
                local.push(format!("rect golden={gr:?} current={cr:?}"));
            }
        }
        if let (Some(gp), Some(cp)) = (g.pivot, c.pivot) {
            if (gp.0 - cp.0).abs() > PIVOT_TOL || (gp.1 - cp.1).abs() > PIVOT_TOL {
                local.push(format!("pivot golden={gp:?} current={cp:?}"));
            }
        }
        if let (Some(go), Some(co)) = (g.offset, c.offset) {
            if (go.0 - co.0).abs() > OFFSET_TOL || (go.1 - co.1).abs() > OFFSET_TOL {
                local.push(format!("offset golden={go:?} current={co:?}"));
            }
        }
        if let (Some(gv), Some(cv)) = (&g.verts, &c.verts) {
            if gv.len() != cv.len() {
                local.push(format!("vert-count golden={} current={}", gv.len(), cv.len()));
            } else {
                let mut bad = 0usize;
                let mut max_ulp = 0i32;
                for ((gx, gy), (cx, cy)) in gv.iter().zip(cv.iter()) {
                    let ux = ulp_diff(*gx, *cx);
                    let uy = ulp_diff(*gy, *cy);
                    max_ulp = max_ulp.max(ux).max(uy);
                    if ux > ULP_TOL || uy > ULP_TOL { bad += 1; }
                }
                if bad > 0 {
                    local.push(format!("verts: {bad}/{} exceed +/-{ULP_TOL} ULP (max={max_ulp})",
                                       gv.len()));
                }
            }
        }
        for issue in &local {
            failures.push(format!("{sprite}: {issue}"));
        }
    }
    assert!(failures.is_empty(), "fixture diff failures:\n  {}", failures.join("\n  "));
}

// ---- Sprite YAML parsing (minimal — pulls just what we diff). -----------

struct Sprite {
    rect:   Option<(f32, f32)>,
    pivot:  Option<(f32, f32)>,
    offset: Option<(f32, f32)>,
    verts:  Option<Vec<(f32, f32)>>,
}

fn parse_asset(text: &str) -> Sprite {
    Sprite {
        rect: parse_width_height(text),
        pivot: parse_xy_after(text, "m_Pivot:"),
        offset: parse_xy_after(text, "m_Offset:"),
        verts: parse_typelessdata(text).map(|hex| {
            let bytes = hex_to_bytes(&hex);
            let mut out = Vec::with_capacity(bytes.len() / 12);
            for chunk in bytes.chunks_exact(12) {
                let x = f32::from_le_bytes(chunk[0..4].try_into().unwrap());
                let y = f32::from_le_bytes(chunk[4..8].try_into().unwrap());
                out.push((x, y));
            }
            out
        }),
    }
}

// width: N\n[\s+]height: M  — first occurrence; the .asset has m_Rect first.
fn parse_width_height(text: &str) -> Option<(f32, f32)> {
    let w_idx = text.find("width: ")?;
    let after_w = &text[w_idx + "width: ".len()..];
    let w_end = after_w.find('\n')?;
    let w: f32 = after_w[..w_end].trim().parse().ok()?;
    let rest = &after_w[w_end..];
    let h_idx = rest.find("height: ")?;
    let after_h = &rest[h_idx + "height: ".len()..];
    let h_end = after_h.find('\n')?;
    let h: f32 = after_h[..h_end].trim().parse().ok()?;
    Some((w, h))
}

// `<label> {x: <a>, y: <b>}` — first occurrence.
fn parse_xy_after(text: &str, label: &str) -> Option<(f32, f32)> {
    let idx = text.find(label)?;
    let line_end = text[idx..].find('\n').unwrap_or(text.len() - idx);
    let line = &text[idx..idx + line_end];
    let x_idx = line.find("x: ")?;
    let after_x = &line[x_idx + 3..];
    let x_end = after_x.find(',')?;
    let x: f32 = after_x[..x_end].trim().parse().ok()?;
    let rest = &after_x[x_end + 1..];
    let y_idx = rest.find("y: ")?;
    let after_y = &rest[y_idx + 3..];
    let y_end = after_y.find('}')?;
    let y: f32 = after_y[..y_end].trim().parse().ok()?;
    Some((x, y))
}

// `_typelessdata: <hex>` — value runs to end of line.
fn parse_typelessdata(text: &str) -> Option<String> {
    let idx = text.find("_typelessdata:")?;
    let after = &text[idx + "_typelessdata:".len()..];
    let end = after.find('\n').unwrap_or(after.len());
    Some(after[..end].trim().to_string())
}

fn hex_to_bytes(s: &str) -> Vec<u8> {
    (0..s.len()/2)
        .map(|i| u8::from_str_radix(&s[i*2..i*2+2], 16).unwrap_or(0))
        .collect()
}

fn ulp_diff(a: f32, b: f32) -> i32 {
    let ba = a.to_bits() as i32;
    let bb = b.to_bits() as i32;
    let na = if ba < 0 { i32::MIN.wrapping_sub(ba) } else { ba };
    let nb = if bb < 0 { i32::MIN.wrapping_sub(bb) } else { bb };
    (na - nb).abs()
}
