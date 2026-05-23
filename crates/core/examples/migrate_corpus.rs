// Offline corpus migration runner — no Unity Editor required.
//
// Walks every `<atlas>.tps` under a given Assets/ root, regenerates the
// sibling `.tpsheet` via TexturePackerCLI (skipping `Template~/` dirs that
// Unity ignores), reads the prefix from `<atlas>.tps.meta`, then calls
// `unity_sprite_author::pipeline::generate` directly (ppu is read from
// `<atlas>.png.meta` inside the pipeline). Same emit semantics as Unity's
// TPSheetPostprocessor, just without the Editor in the loop.
//
// Usage:
//   cargo run --release --example migrate_corpus -- $MEOW_CLIENT/Assets
//
// Idempotent: pipeline deletes the .tpsheet on success, so re-running on
// a clean state regenerates everything.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use unity_sprite_author::pipeline::{self, GenerateInputs};

fn read_meta_field(path: &Path, key: &str) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    for line in text.lines() {
        let line = line.trim_start();
        if let Some(rest) = line.strip_prefix(key) {
            let rest = rest.trim_start_matches(':').trim();
            return Some(rest.to_string());
        }
    }
    None
}

fn read_prefix(tps_meta: &Path) -> String {
    read_meta_field(tps_meta, "_prefix").unwrap_or_default()
}

fn texturepacker(tps: &Path) -> Result<(), String> {
    let dir = tps.parent().ok_or("tps has no parent")?;
    let name = tps.file_name().ok_or("tps has no name")?;
    let out = Command::new("texturepacker")
        .current_dir(dir)
        .arg(name)
        .output()
        .map_err(|e| format!("spawn texturepacker: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    Ok(())
}

fn process_one(tps: &Path) -> Result<(), String> {
    let base = tps.with_extension("");
    let tpsheet = base.with_extension("tpsheet");
    let png = base.with_extension("png");
    let png_meta = base.with_file_name(format!(
        "{}.png.meta",
        base.file_name().unwrap().to_string_lossy()
    ));
    let tps_meta = base.with_file_name(format!(
        "{}.tps.meta",
        base.file_name().unwrap().to_string_lossy()
    ));
    let sprite_dir = base.clone();

    texturepacker(tps).map_err(|e| format!("texturepacker on {}: {e}", tps.display()))?;

    let prefix = read_prefix(&tps_meta);
    pipeline::generate(&GenerateInputs {
        tpsheet_path: &tpsheet,
        tps_path: tps,
        atlas_png_path: &png,
        sprite_dir: &sprite_dir,
        prefix: &prefix,
    })
    .map_err(|e| format!("pipeline::generate on {}: {e}", tps.display()))?;
    Ok(())
}

fn walk_tps_files(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        // Skip Unity-ignored Template~/ dirs.
        if p.file_name()
            .map(|n| n.to_string_lossy().ends_with('~'))
            .unwrap_or(false)
        {
            continue;
        }
        if p.is_dir() {
            walk_tps_files(&p, out);
        } else if p.extension().and_then(|e| e.to_str()) == Some("tps") {
            out.push(p);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = args.get(1).cloned().unwrap_or_else(|| ".".into());
    let root = PathBuf::from(root);

    let mut tps_files = Vec::new();
    walk_tps_files(&root, &mut tps_files);
    tps_files.sort();
    eprintln!("found {} .tps files under {}", tps_files.len(), root.display());

    let ok = AtomicUsize::new(0);
    let mut failures: HashMap<PathBuf, String> = HashMap::new();
    for tps in &tps_files {
        match process_one(tps) {
            Ok(()) => {
                ok.fetch_add(1, Ordering::Relaxed);
            }
            Err(e) => {
                failures.insert(tps.clone(), e);
            }
        }
    }

    let ok = ok.load(Ordering::Relaxed);
    eprintln!("\n--- summary ---");
    eprintln!("ok:     {ok}");
    eprintln!("failed: {}", failures.len());
    if !failures.is_empty() {
        eprintln!("\nfailures:");
        let mut keys: Vec<_> = failures.keys().collect();
        keys.sort();
        for k in keys {
            eprintln!("  {}: {}", k.display(), failures[k]);
        }
        std::process::exit(1);
    }
}
