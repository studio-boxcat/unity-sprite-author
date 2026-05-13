// One-shot CSA → fab.json migration tool. Consumes the text dump emitted by
// `examples/csa_dumper.cs` (run via meow-tower `just scratch "$(cat …)"`),
// groups prefabs by atlas GUID, and writes one `<atlas>.tps.fab.json` per atlas.
//
// Usage:
//   1. From `$MEOW_CLIENT`: `just scratch "$(cat path/to/csa_dumper.cs)"`
//      Writes /tmp/csa-dump.txt with per-leaf Unity probe data.
//   2. `cargo run --example csa_dump_to_fab -- /tmp/csa-dump.txt /tmp/fab-out`
//      Emits one `atlas-<guid>.tps.fab.json` per atlas under `/tmp/fab-out`.
//   3. Caller places each `.tps.fab.json` next to the matching `<atlas>.tps`
//      in meow-tower, runs the pipeline, byte-diffs the emitted `_sprite`
//      `.asset` against the committed golden.
//
// Per-leaf mapping (validated end-to-end via `cargo test --test golden_fab_silloutte`
// after swapping the migrated fab.json over the hand-authored oracle; all 3
// Silloutte byte-exact tests pass):
// - UISolid (script guid 8b6c80…) → polygon part. `polygonSprite` is the leaf's
//   `_sprite.name` minus the atlas `_prefix` (CSA's `ReplaceColorTextures`
//   bakes the color into the sprite, so `g.color` is white post-bake and
//   doesn't carry the color signal). Vertices = pivot-relative quad in
//   BL/BR/TL/TR order; triangles = [0,2,3,3,1,0]; offset = root-relative
//   anchored position.
// - UIIcon (4cc475…) → atlas-sprite part, native scale. `method` from
//   UIMeshMode (ID/MX/MY/MXY); FX/FY/FXY desugar to ID + negative sx/sy per
//   fab.md. `uiScale` = `_scaleFactor`; `offset` = anchored.
// - UISlice (94e8bd…) → atlas-sprite part, size-fitted. `method` from
//   UISliceMethod enum codes; `width`/`height` = sizeDelta; `borderMult`
//   if != 1.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::PathBuf;

const UISOLID_GUID: &str = "8b6c807e975442f39d10d3c228b980c2";
const UIICON_GUID: &str = "4cc47562242545eeb3ec157e7fe0c196";
const UISLICE_GUID: &str = "94e8bd4a79a04bf2a46d57dafc31dac2";

#[derive(Debug, Default)]
struct PrefabDump {
    #[allow(dead_code)]
    prefab_path: String,
    output_sprite_path: String,
    atlas_png_guid: String,
    scale_factor: f32,
    root_anchored: [f32; 2],
    leaves: Vec<Leaf>,
}

#[derive(Debug, Default)]
struct Leaf {
    #[allow(dead_code)]
    name: String,
    script_guid: String,
    sprite_name: String,
    scale_factor: f32,
    method_name: String,
    method_val: i32,
    border_mult: f32,
    color: [f32; 4],
    anchored: [f32; 2],
    size_delta: [f32; 2],
    pivot: [f32; 2],
    rel_m03: f32,
    rel_m13: f32,
}

fn parse_floats<const N: usize>(s: &str) -> [f32; N] {
    let mut out = [0.0_f32; N];
    for (i, tok) in s.split(',').take(N).enumerate() {
        out[i] = tok.parse().unwrap_or(0.0);
    }
    out
}

fn parse_dump(text: &str) -> Vec<PrefabDump> {
    let mut prefabs = Vec::new();
    let mut cur: Option<PrefabDump> = None;
    let mut leaf: Option<Leaf> = None;

    let flush_leaf = |cur: &mut Option<PrefabDump>, leaf: &mut Option<Leaf>| {
        if let (Some(p), Some(l)) = (cur.as_mut(), leaf.take()) {
            p.leaves.push(l);
        }
    };

    for line in text.lines() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if let Some(rest) = trimmed.strip_prefix("PREFAB ") {
            flush_leaf(&mut cur, &mut leaf);
            if let Some(p) = cur.take() {
                prefabs.push(p);
            }
            cur = Some(PrefabDump {
                prefab_path: rest.to_string(),
                ..Default::default()
            });
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("LEAF ") {
            flush_leaf(&mut cur, &mut leaf);
            leaf = Some(Leaf {
                name: rest.to_string(),
                ..Default::default()
            });
            continue;
        }
        let Some(eq) = trimmed.find('=') else { continue };
        let key = &trimmed[..eq];
        let val = &trimmed[eq + 1..];
        if indent == 2 {
            // prefab-level field
            let Some(p) = cur.as_mut() else { continue };
            match key {
                "output_sprite_path" => p.output_sprite_path = val.to_string(),
                "atlas_png_guid" => p.atlas_png_guid = val.to_string(),
                "scale_factor" => p.scale_factor = val.parse().unwrap_or(0.0),
                "root_anchored" => p.root_anchored = parse_floats(val),
                _ => {}
            }
        } else if indent == 4 {
            let Some(l) = leaf.as_mut() else { continue };
            match key {
                "script_guid" => l.script_guid = val.to_string(),
                "sprite_name" => l.sprite_name = val.to_string(),
                "scale_factor" => l.scale_factor = val.parse().unwrap_or(0.0),
                "border_mult" => l.border_mult = val.parse().unwrap_or(1.0),
                "color" => l.color = parse_floats(val),
                "anchored" => l.anchored = parse_floats(val),
                "size_delta" => l.size_delta = parse_floats(val),
                "pivot" => l.pivot = parse_floats(val),
                "rel_m03" => {
                    // value before " bits=..."
                    let v = val.split_whitespace().next().unwrap_or("0");
                    l.rel_m03 = v.parse().unwrap_or(0.0);
                }
                "rel_m13" => {
                    let v = val.split_whitespace().next().unwrap_or("0");
                    l.rel_m13 = v.parse().unwrap_or(0.0);
                }
                k if k.starts_with("method_name") => {
                    // line is "method_name=NAME method_val=NUM"
                    let mut parts = val.split_whitespace();
                    l.method_name = parts.next().unwrap_or("").to_string();
                    if let Some(mv) = parts.next() {
                        if let Some(num) = mv.strip_prefix("method_val=") {
                            l.method_val = num.parse().unwrap_or(-1);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    flush_leaf(&mut cur, &mut leaf);
    if let Some(p) = cur.take() {
        prefabs.push(p);
    }
    prefabs
}

fn polygon_vertices(size: [f32; 2], pivot: [f32; 2]) -> [[f32; 2]; 4] {
    let (w, h) = (size[0], size[1]);
    let (px, py) = (pivot[0], pivot[1]);
    let xmin = -px * w;
    let xmax = (1.0 - px) * w;
    let ymin = -py * h;
    let ymax = (1.0 - py) * h;
    // BL, BR, TL, TR — triangle list [0,2,3,3,1,0] in fab.json.
    [[xmin, ymin], [xmax, ymin], [xmin, ymax], [xmax, ymax]]
}

fn strip_prefix_guess<'a>(sprite_name: &'a str) -> &'a str {
    // Sprite names dump-side carry the atlas `_prefix` (e.g. "PE_33_Mansion_…").
    // fab.json wants the bare tpsheet entry name. We don't have the atlas's
    // `.tps.meta` here, so strip the longest leading `[A-Z0-9]+_` run that
    // matches a known prefix shape. Caller can override per-atlas if needed.
    // For Silloutte / meow-tower convention: prefix is uppercase + digits + '_'.
    let mut last_us = 0usize;
    for (i, &b) in sprite_name.as_bytes().iter().enumerate() {
        if b == b'_' {
            last_us = i + 1;
        } else if !(b.is_ascii_uppercase() || b.is_ascii_digit()) {
            break;
        }
    }
    &sprite_name[last_us..]
}

fn fmt_f(v: f32) -> String {
    // Match the oracle's number style: minimal decimal, no scientific notation,
    // no trailing zeros after the decimal point unless needed.
    if v == v.trunc() && v.abs() < 1e16 {
        format!("{}", v as i64)
    } else {
        let s = format!("{}", v);
        s
    }
}

fn emit_part_polygon(l: &Leaf) -> String {
    let verts = polygon_vertices(l.size_delta, l.pivot);
    let mut s = String::new();
    s.push_str("        { \"polygonSprite\": \"");
    s.push_str(strip_prefix_guess(&l.sprite_name));
    s.push_str("\",\n          \"vertices\": [");
    for (i, v) in verts.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        s.push_str(&format!("[{}, {}]", fmt_f(v[0]), fmt_f(v[1])));
    }
    s.push_str("],\n          \"triangles\": [0, 2, 3, 3, 1, 0]");
    let off = [l.anchored[0], l.anchored[1]];
    if off != [0.0, 0.0] {
        s.push_str(&format!(",\n          \"offset\": [{}, {}]", fmt_f(off[0]), fmt_f(off[1])));
    }
    s.push_str(" }");
    s
}

fn ui_mesh_mode_name(val: i32) -> (&'static str, f32, f32) {
    // UIMeshMode: ID=0, MX=1, MY=2, MXY=3, FX=4, FY=5, FXY=6
    // FX/FY/FXY desugar to ID + negative sx/sy per fab.md.
    match val {
        0 => ("ID", 1.0, 1.0),
        1 => ("MX", 1.0, 1.0),
        2 => ("MY", 1.0, 1.0),
        3 => ("MXY", 1.0, 1.0),
        4 => ("ID", -1.0, 1.0),
        5 => ("ID", 1.0, -1.0),
        6 => ("ID", -1.0, -1.0),
        _ => ("ID", 1.0, 1.0),
    }
}

fn ui_slice_method_name(val: i32) -> &'static str {
    // UISliceMethod codes — see UISlice.cs.
    match val {
        9 => "ID",
        10 => "FX",
        11 => "FY",
        12 => "FXY",
        0 => "MX",
        17 => "MY",
        3 => "MXY",
        23 => "TX",
        24 => "TY",
        25 => "TX_MC3",
        15 => "R1C3",
        13 => "R3C3",
        18 => "R3C3_NF",
        7 => "MX_R1C3",
        1 => "MX_R1C4",
        8 => "MX_R3C2",
        6 => "MX_R3C3",
        2 => "MX_R3C4",
        4 => "MX_R3C6",
        22 => "MY_R2C2",
        16 => "MY_R2C3",
        20 => "MY_R3C1",
        21 => "MY_R3C2",
        14 => "MY_R3C3",
        5 => "MXY_R3C3",
        19 => "MXY_R3C3_NF",
        _ => "ID",
    }
}

fn emit_part_atlas_native(l: &Leaf) -> String {
    let (method, sx, sy) = ui_mesh_mode_name(l.method_val);
    let bare_sprite = strip_prefix_guess(&l.sprite_name);
    let mut s = String::new();
    s.push_str(&format!("        {{ \"sprite\": \"{}\", \"method\": \"{}\"", bare_sprite, method));
    if sx != 1.0 {
        s.push_str(&format!(", \"sx\": {}", fmt_f(sx)));
    }
    if sy != 1.0 {
        s.push_str(&format!(", \"sy\": {}", fmt_f(sy)));
    }
    s.push_str(&format!(",\n          \"uiScale\": {}", fmt_f(l.scale_factor)));
    s.push_str(&format!(", \"offset\": [{}, {}] }}", fmt_f(l.anchored[0]), fmt_f(l.anchored[1])));
    s
}

fn emit_part_atlas_slice(l: &Leaf) -> String {
    let method = ui_slice_method_name(l.method_val);
    let bare_sprite = strip_prefix_guess(&l.sprite_name);
    let mut s = String::new();
    s.push_str(&format!("        {{ \"sprite\": \"{}\", \"method\": \"{}\"", bare_sprite, method));
    s.push_str(&format!(",\n          \"width\": {}, \"height\": {}", fmt_f(l.size_delta[0]), fmt_f(l.size_delta[1])));
    s.push_str(&format!(",\n          \"uiScale\": {}", fmt_f(l.scale_factor)));
    s.push_str(&format!(", \"offset\": [{}, {}]", fmt_f(l.anchored[0]), fmt_f(l.anchored[1])));
    if (l.border_mult - 1.0).abs() > 1e-6 {
        s.push_str(&format!(", \"borderMult\": {}", fmt_f(l.border_mult)));
    }
    s.push_str(" }");
    s
}

fn emit_part(l: &Leaf) -> Option<String> {
    match l.script_guid.as_str() {
        UISOLID_GUID => Some(emit_part_polygon(l)),
        UIICON_GUID => Some(emit_part_atlas_native(l)),
        UISLICE_GUID => Some(emit_part_atlas_slice(l)),
        _ => None,
    }
}

fn output_name_from_path(p: &str) -> String {
    PathBuf::from(p)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string()
}

fn emit_combined(p: &PrefabDump) -> String {
    let name = output_name_from_path(&p.output_sprite_path);
    let mut s = String::new();
    s.push_str("    {\n");
    s.push_str(&format!("      \"name\": \"{}\",\n", name));
    s.push_str(&format!("      \"canvasScale\": {},\n", fmt_f(p.scale_factor)));
    if p.root_anchored != [0.0, 0.0] {
        s.push_str(&format!(
            "      \"rootAnchored\": [{}, {}],\n",
            fmt_f(p.root_anchored[0]),
            fmt_f(p.root_anchored[1])
        ));
    }
    s.push_str("      \"parts\": [\n");
    let parts: Vec<String> = p.leaves.iter().filter_map(emit_part).collect();
    s.push_str(&parts.join(",\n"));
    s.push_str("\n      ]\n    }");
    s
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let dump_path = args.get(1).cloned().unwrap_or_else(|| "/tmp/csa-dump.txt".into());
    let text = fs::read_to_string(&dump_path).expect("read dump");
    let prefabs = parse_dump(&text);

    // Group by atlas GUID; key kept stable via BTreeMap.
    let mut by_atlas: BTreeMap<String, Vec<&PrefabDump>> = BTreeMap::new();
    for p in &prefabs {
        by_atlas.entry(p.atlas_png_guid.clone()).or_default().push(p);
    }

    let out_dir = args.get(2).map(PathBuf::from);
    for (atlas_guid, prefabs) in &by_atlas {
        let mut s = String::new();
        s.push_str("{\n  \"version\": 1,\n  \"combined\": [\n");
        let blocks: Vec<String> = prefabs.iter().map(|p| emit_combined(p)).collect();
        s.push_str(&blocks.join(",\n"));
        s.push_str("\n  ]\n}\n");
        if let Some(dir) = &out_dir {
            fs::create_dir_all(dir).expect("mkdir out_dir");
            let path = dir.join(format!("atlas-{}.tps.fab.json", atlas_guid));
            fs::write(&path, &s).expect("write fab.json");
            eprintln!("wrote {} ({} prefabs)", path.display(), prefabs.len());
        } else {
            eprintln!("=== ATLAS {} ({} prefabs) ===", atlas_guid, prefabs.len());
            println!("{}", s);
        }
    }
}
