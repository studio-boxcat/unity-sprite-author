// Bench harness covering the full pipeline plus per-stage hot paths.
// Run: cargo bench
// Flamegraph: cargo flamegraph --bench pipeline -- --bench

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};

use unity_sprite_author::{
    emit::{self, SpriteAsset},
    meta,
    pipeline::{self, GenerateInputs},
    render_data::{self, AtlasSize},
    tpsheet,
    yaml,
};

const ATLAS_PPU: f32 = 80.0;
const ATLAS_SIZE: AtlasSize = AtlasSize {
    width: 580,
    height: 580,
};

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/orgel")
}

fn copy_dir(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn stage_orgel(suffix: &str) -> PathBuf {
    let dst = std::env::temp_dir().join(format!("uspa_bench_{suffix}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dst);
    copy_dir(&fixture_root(), &dst).unwrap();
    dst
}

fn bench_pipeline_full(c: &mut Criterion) {
    // Fixture has 62 sprites. Setup copies the directory each iteration
    // because pipeline::generate deletes the .tpsheet on success and rewrites
    // the .asset siblings. iter_batched factors setup time out of the
    // measurement.
    let mut counter = 0u64;
    c.bench_function("pipeline_generate_orgel_62_sprites", |b| {
        b.iter_batched(
            || {
                counter += 1;
                stage_orgel(&format!("full_{counter}"))
            },
            |dir| {
                let inputs = GenerateInputs {
                    tpsheet_path: &dir.join("Orgel.tpsheet"),
                    tps_path: &dir.join("Orgel.tps"),
                    atlas_png_path: &dir.join("Orgel.png"),
                    sprite_dir: &dir.join("sprites"),
                    prefix: "",
                    ppu: ATLAS_PPU,
                };
                let out = pipeline::generate(&inputs).unwrap();
                let _ = fs::remove_dir_all(&dir);
                out
            },
            BatchSize::PerIteration,
        );
    });
}

fn bench_tpsheet_parse(c: &mut Criterion) {
    let text = fs::read_to_string(fixture_root().join("Orgel.tpsheet")).unwrap();
    c.bench_function("tpsheet_parse_orgel", |b| {
        b.iter(|| tpsheet::parse(&text).unwrap());
    });
}

fn bench_render_data_build(c: &mut Criterion) {
    let text = fs::read_to_string(fixture_root().join("Orgel.tpsheet")).unwrap();
    let sheet = tpsheet::parse(&text).unwrap();
    let sprite = sheet
        .sprites
        .iter()
        .find(|s| s.name == "Cake__DecoLeft")
        .unwrap()
        .clone();
    c.bench_function("render_data_build_cake_decoleft", |b| {
        b.iter(|| {
            render_data::build(
                sprite.rect,
                sprite.pivot,
                &sprite.geometry.vertices,
                &sprite.geometry.triangles,
                ATLAS_PPU,
                1.0,
                ATLAS_SIZE,
            )
        });
    });
}

fn bench_emit_one_sprite(c: &mut Criterion) {
    let text = fs::read_to_string(fixture_root().join("Orgel.tpsheet")).unwrap();
    let sheet = tpsheet::parse(&text).unwrap();
    let sprite = sheet
        .sprites
        .iter()
        .find(|s| s.name == "Cake__DecoLeft")
        .unwrap()
        .clone();
    let atlas_meta = fs::read_to_string(fixture_root().join("Orgel.png.meta")).unwrap();
    let atlas_guid = meta::parse_guid(&atlas_meta).unwrap();
    let own_meta = fs::read_to_string(
        fixture_root().join("sprites/Cake__DecoLeft.asset.meta"),
    )
    .unwrap();
    let own_guid = meta::parse_guid(&own_meta).unwrap();
    let rd = render_data::build(
        sprite.rect,
        sprite.pivot,
        &sprite.geometry.vertices,
        &sprite.geometry.triangles,
        ATLAS_PPU,
        1.0,
        ATLAS_SIZE,
    );
    let asset = SpriteAsset {
        name: sprite.name.clone(),
        rect: sprite.rect,
        border: sprite.border,
        pivot: sprite.pivot,
        pixels_to_units: ATLAS_PPU,
        own_guid,
        atlas_guid,
        render_data: rd,
    };
    c.bench_function("emit_cake_decoleft", |b| {
        b.iter(|| emit::emit(&asset));
    });
}

fn bench_meta_render(c: &mut Criterion) {
    let guid = [
        0xd4, 0xc7, 0x82, 0xeb, 0x33, 0x40, 0xc4, 0x18, 0x48, 0xb2, 0xa0, 0xa9, 0x03, 0xc0, 0xfc,
        0xea,
    ];
    c.bench_function("meta_render_asset_meta", |b| {
        b.iter(|| meta::render_asset_meta(&guid));
    });
}

fn bench_guid_hex(c: &mut Criterion) {
    let guid = [0xab; 16];
    c.bench_function("yaml_guid_hex", |b| {
        b.iter(|| yaml::guid_hex(&guid));
    });
}

criterion_group!(
    benches,
    bench_pipeline_full,
    bench_tpsheet_parse,
    bench_render_data_build,
    bench_emit_one_sprite,
    bench_meta_render,
    bench_guid_hex,
);
criterion_main!(benches);
