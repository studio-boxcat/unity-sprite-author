// One-shot SMA → `.tps.mesh.json` converter. Consumes the text dump emitted
// by `examples/sma_dumper.cs` (run via meow-tower `just scratch …`),
// groups SMAs by atlas GUID + output asset path, and writes one
// `<atlas>.tps.mesh.json` per atlas under the output dir.
//
// Usage:
//   1. From `$MEOW_CLIENT`: `just scratch "$(cat path/to/sma_dumper.cs)"`
//      Writes /tmp/sma-dump.txt with per-SMA + per-leaf Unity probe data.
//   2. `cargo run --example sma_dump_to_mesh_manifest -- /tmp/sma-dump.txt /tmp/mesh-out`
//      Emits one `atlas-<guid>.tps.mesh.json` per atlas under `/tmp/mesh-out`.
//   3. Caller places each `.tps.mesh.json` next to the matching `<atlas>.tps`
//      in meow-tower, runs the pipeline, byte-diffs the emitted Mesh
//      `.asset` against a fresh `CSA/SMA.Publish()` capture.
//
// Mapping (mirrors `examples/sma_dumper.cs` output → `mesh_manifest::MeshCombined`
// field-for-field; the dumper already captures everything the parser needs):
//   SMA header → MeshCombined {
//     file_id: mesh_file_id, name: sma_name, output_path: output_asset_path,
//     used_in_canvas, keep_vertices=1, keep_indices=1,
//     renderers: [each LEAF → MeshRenderer {...}]
//   }
//
// `output_path` is rewritten to be relative to the manifest's directory
// (the atlas dir), so a Box prefab whose Mesh lives at
// `Assets/21_Collections/Boxes/!Output/Box_29_Ghost.asset` resolves to
// `../!Output/Box_29_Ghost.asset` when the atlas is at
// `Assets/21_Collections/Boxes/{atlas}.tps`.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
struct SmaEntry {
    #[allow(dead_code)]
    prefab_path: String,
    output_asset_path: String,
    mesh_file_id: i64,
    used_in_canvas: bool,
    atlas_png_guid: String,
    atlas_prefix: String,
    sma_name: String,
    leaves: Vec<LeafEntry>,
}

#[derive(Debug, Default)]
struct LeafEntry {
    #[allow(dead_code)]
    name: String,
    sprite_name: String,
    flip_x: bool,
    flip_y: bool,
    draw_mode: String,
    size: Option<[f32; 2]>,
    l2r: [f32; 8],
}

fn parse_floats<const N: usize>(s: &str) -> [f32; N] {
    let mut out = [0.0_f32; N];
    for (i, tok) in s.split(',').take(N).enumerate() {
        out[i] = tok.trim().parse().unwrap_or(0.0);
    }
    out
}

fn parse_dump(text: &str) -> Vec<SmaEntry> {
    let mut entries = Vec::new();
    let mut cur: Option<SmaEntry> = None;
    let mut leaf: Option<LeafEntry> = None;

    let flush_leaf = |cur: &mut Option<SmaEntry>, leaf: &mut Option<LeafEntry>| {
        if let (Some(c), Some(l)) = (cur.as_mut(), leaf.take()) {
            c.leaves.push(l);
        }
    };

    for line in text.lines() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if let Some(rest) = trimmed.strip_prefix("SMA ") {
            flush_leaf(&mut cur, &mut leaf);
            if let Some(e) = cur.take() {
                entries.push(e);
            }
            cur = Some(SmaEntry {
                prefab_path: rest.to_string(),
                ..Default::default()
            });
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("LEAF ") {
            flush_leaf(&mut cur, &mut leaf);
            leaf = Some(LeafEntry {
                name: rest.to_string(),
                ..Default::default()
            });
            continue;
        }
        let Some(eq) = trimmed.find('=') else { continue };
        let key = &trimmed[..eq];
        let val = &trimmed[eq + 1..];
        if indent == 2 {
            let Some(c) = cur.as_mut() else { continue };
            match key {
                "output_asset_path" => c.output_asset_path = val.to_string(),
                "mesh_file_id" => c.mesh_file_id = val.parse().unwrap_or(0),
                "used_in_canvas" => c.used_in_canvas = val == "1",
                "atlas_png_guid" => c.atlas_png_guid = val.to_string(),
                "atlas_prefix" => c.atlas_prefix = val.to_string(),
                "sma_name" => c.sma_name = val.to_string(),
                _ => {}
            }
        } else if indent == 4 {
            let Some(l) = leaf.as_mut() else { continue };
            match key {
                "sprite_name" => l.sprite_name = val.to_string(),
                "flip_x" => l.flip_x = val == "1",
                "flip_y" => l.flip_y = val == "1",
                "draw_mode" => l.draw_mode = val.to_string(),
                "size" => l.size = Some(parse_floats(val)),
                "l2r" => l.l2r = parse_floats(val),
                _ => {}
            }
        }
    }
    flush_leaf(&mut cur, &mut leaf);
    if let Some(e) = cur.take() {
        entries.push(e);
    }
    entries
}

fn strip_prefix<'a>(sprite_name: &'a str, prefix: &str) -> &'a str {
    sprite_name.strip_prefix(prefix).unwrap_or(sprite_name)
}

fn fmt_f(v: f32) -> String {
    if v == v.trunc() && v.abs() < 1e16 {
        format!("{}", v as i64)
    } else {
        format!("{}", v)
    }
}

fn relative_path(from_dir: &Path, to: &Path) -> PathBuf {
    // Both paths are Assets-relative; compute `to` relative to `from_dir`.
    let from_comps: Vec<_> = from_dir.components().collect();
    let to_comps: Vec<_> = to.components().collect();
    let common = from_comps
        .iter()
        .zip(to_comps.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let mut rel = PathBuf::new();
    for _ in 0..(from_comps.len() - common) {
        rel.push("..");
    }
    for c in &to_comps[common..] {
        rel.push(c);
    }
    rel
}

fn emit_renderer(l: &LeafEntry, prefix: &str) -> String {
    let mut s = String::new();
    let sprite = strip_prefix(&l.sprite_name, prefix);
    s.push_str("          {\n");
    s.push_str(&format!("            \"sprite\": \"{}\",\n", sprite));
    if l.flip_x {
        s.push_str("            \"flipX\": true,\n");
    }
    if l.flip_y {
        s.push_str("            \"flipY\": true,\n");
    }
    s.push_str(&format!("            \"drawMode\": \"{}\",\n", l.draw_mode));
    if let Some(sz) = l.size {
        s.push_str(&format!(
            "            \"size\": [{}, {}],\n",
            fmt_f(sz[0]),
            fmt_f(sz[1])
        ));
    }
    s.push_str("            \"localToRoot\": [");
    for (i, v) in l.l2r.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        s.push_str(&fmt_f(*v));
    }
    s.push_str("]\n");
    s.push_str("          }");
    s
}

fn emit_combined(e: &SmaEntry, manifest_dir: &Path) -> String {
    let out_abs = Path::new(&e.output_asset_path);
    let rel = relative_path(manifest_dir, out_abs);
    let mut s = String::new();
    s.push_str("    {\n");
    s.push_str(&format!("      \"fileId\": {},\n", e.mesh_file_id));
    s.push_str(&format!("      \"name\": \"{}\",\n", e.sma_name));
    s.push_str(&format!(
        "      \"outputPath\": \"{}\",\n",
        rel.display().to_string().replace('\\', "/")
    ));
    s.push_str(&format!(
        "      \"usedInCanvas\": {},\n",
        e.used_in_canvas
    ));
    s.push_str("      \"renderers\": [\n");
    let parts: Vec<String> = e
        .leaves
        .iter()
        .map(|l| emit_renderer(l, &e.atlas_prefix))
        .collect();
    s.push_str(&parts.join(",\n"));
    s.push_str("\n      ]\n");
    s.push_str("    }");
    s
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let dump_path = args.get(1).cloned().unwrap_or_else(|| "/tmp/sma-dump.txt".into());
    let out_dir = args.get(2).cloned().unwrap_or_else(|| "/tmp/mesh-out".into());
    let text = fs::read_to_string(&dump_path).expect("read dump");
    let entries = parse_dump(&text);
    fs::create_dir_all(&out_dir).expect("mkdir out_dir");

    // Group by atlas_png_guid.
    let mut by_atlas: BTreeMap<String, Vec<&SmaEntry>> = BTreeMap::new();
    for e in &entries {
        by_atlas.entry(e.atlas_png_guid.clone()).or_default().push(e);
    }

    for (atlas_guid, group) in &by_atlas {
        if atlas_guid.is_empty() {
            eprintln!(
                "skipping {} SMAs without atlas (likely no sprites)",
                group.len()
            );
            continue;
        }
        // The atlas directory is recovered from each SMA's output_asset_path:
        // we use the first entry's atlas guess via the prefab path's
        // common ancestor with output_asset_path. For real-world cases the
        // user feeds the atlas dir explicitly via the placement step; here
        // we emit with `"outputPath"` relative to a placeholder "atlas_dir"
        // that the caller substitutes when placing the file next to .tps.
        //
        // Convention: the manifest is placed next to `<atlas>.tps`. We
        // resolve `output_path` as relative to that atlas dir at PLACEMENT
        // time by passing `--manifest-dir` (3rd arg). When absent we emit
        // absolute paths and let the caller rewrite.
        let manifest_dir = args
            .get(3)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("Assets"));

        let mut s = String::new();
        s.push_str("{\n  \"version\": 1,\n  \"meshes\": [\n");
        let blocks: Vec<String> = group.iter().map(|e| emit_combined(e, &manifest_dir)).collect();
        s.push_str(&blocks.join(",\n"));
        s.push_str("\n  ]\n}\n");

        let path = PathBuf::from(&out_dir).join(format!("atlas-{}.tps.mesh.json", atlas_guid));
        fs::write(&path, &s).expect("write mesh.json");
        eprintln!("wrote {} ({} SMAs)", path.display(), group.len());
    }
}
