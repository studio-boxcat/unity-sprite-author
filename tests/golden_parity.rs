// Multi-fixture byte-exact parity test. Runs the emit pipeline over every
// sprite in tests/golden/orgel/ where the committed .tps is consistent with
// the committed .asset (i.e., m_PixelsToUnits == 80 → sprite_scale == 1).
//
// The other 54 sprites in Orgel have m_PixelsToUnits != 80, meaning the .tps
// was edited after the .asset goldens were emitted by Unity. Validating those
// requires regenerating the goldens — see TODO.md.

use std::fs;
use std::path::Path;

use unity_sprite_author::{
    emit::{self, SpriteAsset},
    render_data::{self, AtlasSize},
    tpsheet,
};

const ATLAS_PPU: f32 = 80.0;
const ATLAS_SIZE: AtlasSize = AtlasSize {
    width: 580,
    height: 580,
};

fn parse_guid(meta: &str) -> [u8; 16] {
    for line in meta.lines() {
        if let Some(rest) = line.strip_prefix("guid: ") {
            let hex = rest.trim();
            let mut out = [0u8; 16];
            for (i, byte) in out.iter_mut().enumerate() {
                *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
                    .expect("valid hex in guid");
            }
            return out;
        }
    }
    panic!("no guid: line in meta");
}

fn pixels_to_units(asset_text: &str) -> f32 {
    for line in asset_text.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("m_PixelsToUnits: ") {
            return rest.trim().parse().expect("valid m_PixelsToUnits");
        }
    }
    panic!("no m_PixelsToUnits found");
}

#[test]
fn orgel_byte_exact_for_sprite_scale_1() {
    let fixture_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/orgel");
    let sprites_dir = fixture_dir.join("sprites");

    let tpsheet_text = fs::read_to_string(fixture_dir.join("Orgel.tpsheet")).unwrap();
    let atlas_meta_text = fs::read_to_string(fixture_dir.join("Orgel.png.meta")).unwrap();
    let atlas_guid = parse_guid(&atlas_meta_text);
    let sheet = tpsheet::parse(&tpsheet_text).unwrap();

    let mut tested = 0usize;
    let mut skipped = Vec::new();

    for sprite in &sheet.sprites {
        let asset_path = sprites_dir.join(format!("{}.asset", sprite.name));
        let meta_path = sprites_dir.join(format!("{}.asset.meta", sprite.name));
        let golden = match fs::read_to_string(&asset_path) {
            Ok(s) => s,
            Err(_) => panic!("missing golden .asset: {asset_path:?}"),
        };

        let ptu = pixels_to_units(&golden);
        if (ptu - ATLAS_PPU).abs() > 1e-6 {
            // The .tps was edited after this golden was emitted. Skip; tracked in TODO.md.
            skipped.push((sprite.name.clone(), ptu));
            continue;
        }

        let meta_text = fs::read_to_string(&meta_path).unwrap();
        let own_guid = parse_guid(&meta_text);
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
            texture_rect_size: None,
        };
        let got = emit::emit(&asset).expect("emit succeeded");
        assert_eq!(got, golden, "byte mismatch on sprite {}", sprite.name);
        tested += 1;
    }

    assert!(tested >= 8, "expected at least 8 ppu=80 sprites, tested {tested}");
    eprintln!(
        "byte-exact: {tested} sprites tested, {} skipped (drifted .tps)",
        skipped.len()
    );
}
