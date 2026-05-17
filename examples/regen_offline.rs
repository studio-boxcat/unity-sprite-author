// Offline single-atlas regen — no Unity project, no .png file, no
// TexturePackerCLI. Reads tpsheet/tps/png.meta/fab.json/.asset.metas
// already staged on disk and runs `pipeline::generate` against them.
//
// Used by the e2e validation harness that stages meow-tower atlas inputs
// into /tmp, applies fab.json restoration scripts, regenerates, and diffs
// against pre-c23474b2 goldens — without dirtying meow-tower.
//
// Usage:
//   regen_offline --tps <path> --tpsheet <path> --png-meta <path>
//                 --sprite-dir <path> --ppu 100 [--prefix STR]

use std::path::PathBuf;
use unity_sprite_author::pipeline::{self, GenerateInputs};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut tps: Option<PathBuf> = None;
    let mut tpsheet: Option<PathBuf> = None;
    let mut png_meta: Option<PathBuf> = None;
    let mut sprite_dir: Option<PathBuf> = None;
    let mut prefix: String = String::new();
    let mut ppu: f32 = 100.0;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--tps" => tps = Some(it.next().expect("--tps val").into()),
            "--tpsheet" => tpsheet = Some(it.next().expect("--tpsheet val").into()),
            "--png-meta" => png_meta = Some(it.next().expect("--png-meta val").into()),
            "--sprite-dir" => sprite_dir = Some(it.next().expect("--sprite-dir val").into()),
            "--prefix" => prefix = it.next().expect("--prefix val").clone(),
            "--ppu" => ppu = it.next().expect("--ppu val").parse().expect("ppu f32"),
            other => panic!("unknown arg: {other}"),
        }
    }
    let tps = tps.expect("--tps required");
    let tpsheet = tpsheet.expect("--tpsheet required");
    let png_meta = png_meta.expect("--png-meta required");
    let sprite_dir = sprite_dir.expect("--sprite-dir required");

    // Pipeline derives png path from png_meta by stripping `.meta`.
    let png = png_meta.with_extension("");

    pipeline::generate(&GenerateInputs {
        tpsheet_path: &tpsheet,
        tps_path: &tps,
        atlas_png_path: &png,
        sprite_dir: &sprite_dir,
        prefix: &prefix,
        ppu,
    })
    .expect("pipeline");
}
