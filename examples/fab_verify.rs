// Phase 1b byte-diff verifier. Given:
//   --atlas-tps <path>             # e.g. .../PencilAdPopup_InfinitePencil.tps
//   --fab-json <path>              # migrated <atlas>.tps.fab.json (from csa_dump_to_fab)
//   --combined <name>:<sprite-path>...  # per-entry committed _sprite.asset to diff against
//
// The verifier:
//   1. Copies the atlas's `.tps`, `.tpsheet`, `.png.meta`, `.tps.meta` + the
//      fab.json into a temp staging dir.
//   2. Stages each combined entry's existing `.asset.meta` into the staged
//      sprite_dir so `pipeline::generate` hits the GUID-preserve branch.
//   3. Runs `pipeline::generate` against the staged dir.
//   4. Byte-diffs each emitted sprite `.asset` against the supplied committed
//      golden, reports first divergence with hex window.
//
// Doesn't touch meow-tower. Caller copies / regen `.tpsheet` separately.

use std::fs;
use std::path::{Path, PathBuf};

use unity_sprite_author::pipeline::{self, GenerateInputs};

fn first_diff(a: &[u8], b: &[u8]) -> Option<usize> {
    a.iter()
        .zip(b.iter())
        .position(|(x, y)| x != y)
        .or_else(|| if a.len() != b.len() { Some(a.len().min(b.len())) } else { None })
}

fn copy_into(src: &Path, dst_dir: &Path) {
    let dst = dst_dir.join(src.file_name().unwrap());
    fs::copy(src, &dst).unwrap_or_else(|e| panic!("copy {}: {e}", src.display()));
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut atlas_tps: Option<PathBuf> = None;
    let mut fab_json: Option<PathBuf> = None;
    let mut combined: Vec<(String, PathBuf)> = Vec::new();
    let mut ppu = 100.0_f32;
    let mut prefix = String::new();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--atlas-tps" => { atlas_tps = Some(PathBuf::from(&args[i + 1])); i += 2; }
            "--fab-json" => { fab_json = Some(PathBuf::from(&args[i + 1])); i += 2; }
            "--combined" => {
                let nv = &args[i + 1];
                let (name, path) = nv.split_once(':').expect("--combined NAME:PATH");
                combined.push((name.to_string(), PathBuf::from(path)));
                i += 2;
            }
            "--ppu" => { ppu = args[i + 1].parse().unwrap(); i += 2; }
            "--prefix" => { prefix = args[i + 1].clone(); i += 2; }
            other => panic!("unknown arg: {other}"),
        }
    }

    let atlas_tps = atlas_tps.expect("--atlas-tps required");
    let fab_json = fab_json.expect("--fab-json required");
    let atlas_dir = atlas_tps.parent().unwrap();
    let atlas_stem = atlas_tps.file_stem().unwrap().to_str().unwrap();

    let stage = std::env::temp_dir().join(format!("uspa_verify_{atlas_stem}"));
    let _ = fs::remove_dir_all(&stage);
    fs::create_dir_all(&stage).unwrap();
    let sprite_dir = stage.join(atlas_stem);
    fs::create_dir_all(&sprite_dir).unwrap();

    // Stage atlas inputs.
    for sib in [
        format!("{atlas_stem}.tps"),
        format!("{atlas_stem}.tpsheet"),
        format!("{atlas_stem}.png"),
        format!("{atlas_stem}.png.meta"),
        format!("{atlas_stem}.tps.meta"),
    ] {
        let src = atlas_dir.join(&sib);
        if src.exists() {
            copy_into(&src, &stage);
        }
    }
    let fab_dst = stage.join(format!("{atlas_stem}.tps.fab.json"));
    fs::copy(&fab_json, &fab_dst).unwrap();

    // Stage existing _sprite.asset.meta inside sprite_dir so GUID preserve fires.
    for (name, asset_path) in &combined {
        let meta = asset_path.with_extension("asset.meta");
        if meta.exists() {
            fs::copy(&meta, sprite_dir.join(format!("{name}.asset.meta"))).unwrap();
        }
    }

    let inputs = GenerateInputs {
        tpsheet_path: &stage.join(format!("{atlas_stem}.tpsheet")),
        tps_path: &stage.join(format!("{atlas_stem}.tps")),
        atlas_png_path: &stage.join(format!("{atlas_stem}.png")),
        sprite_dir: &sprite_dir,
        prefix: &prefix,
        ppu,
    };
    let res = pipeline::generate(&inputs);
    match res {
        Ok(out) => eprintln!("pipeline ok: {} written, {} deleted", out.written_paths.len(), out.deleted_paths.len()),
        Err(e) => { eprintln!("pipeline FAILED: {e:?}"); std::process::exit(2); }
    }

    let mut pass = 0;
    let mut fail = 0;
    for (name, golden_path) in &combined {
        let emitted = sprite_dir.join(format!("{name}.asset"));
        let got = match fs::read(&emitted) {
            Ok(b) => b,
            Err(e) => { println!("MISS {name}: emitted file missing: {e}"); fail += 1; continue; }
        };
        let golden = fs::read(golden_path).unwrap();
        if got == golden {
            println!("OK   {name} ({} bytes)", got.len());
            pass += 1;
        } else {
            let off = first_diff(&got, &golden).unwrap();
            println!("DIFF {name} at offset {off} (got {} bytes, golden {} bytes)", got.len(), golden.len());
            let _ = fs::create_dir_all("target/diff");
            let _ = fs::write(format!("target/diff/{name}.actual"), &got);
            let _ = fs::write(format!("target/diff/{name}.expected"), &golden);
            fail += 1;
        }
    }
    println!("--- {pass} pass / {fail} fail ---");
    std::process::exit(if fail == 0 { 0 } else { 1 });
}
