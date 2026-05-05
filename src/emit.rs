// Emit a Unity Sprite .asset byte-exactly. Hand-rolled (no serde_yaml) to
// pin every whitespace/format quirk: trailing spaces on m_PackingTag and
// m_SpriteID, single LF at EOF, _typelessdata as one unbroken hex line,
// m_RenderDataKey as the only non-flow nested mapping.

use std::fmt::Write;

use crate::render_data::RenderData;
use crate::tpsheet::{Border, Pivot, Rect};
use crate::yaml::{float, guid_hex};

#[derive(Debug, Clone)]
pub struct SpriteAsset {
    pub name: String,
    pub rect: Rect,
    pub border: Border,
    pub pivot: Pivot,
    pub pixels_to_units: f32,
    pub own_guid: [u8; 16],   // also written to m_RenderDataKey
    pub atlas_guid: [u8; 16], // texture reference inside m_RD/m_AtlasRD
    pub render_data: RenderData,
}

pub fn emit(asset: &SpriteAsset) -> String {
    let mut s = String::with_capacity(4096);

    // Header — fixed.
    s.push_str("%YAML 1.1\n");
    s.push_str("%TAG !u! tag:unity3d.com,2011:\n");
    s.push_str("--- !u!213 &21300000\n");
    s.push_str("Sprite:\n");

    // Sprite mapping body, 2-space indent.
    s.push_str("  m_ObjectHideFlags: 0\n");
    s.push_str("  m_CorrespondingSourceObject: {fileID: 0}\n");
    s.push_str("  m_PrefabInstance: {fileID: 0}\n");
    s.push_str("  m_PrefabAsset: {fileID: 0}\n");
    writeln!(s, "  m_Name: {}", asset.name).unwrap();

    // m_Rect block.
    s.push_str("  m_Rect:\n");
    s.push_str("    serializedVersion: 2\n");
    write_rect_fields(&mut s, "    ", asset.rect);

    s.push_str("  m_Offset: {x: 0, y: 0}\n");
    writeln!(
        s,
        "  m_Border: {{x: {}, y: {}, z: {}, w: {}}}",
        asset.border.left, asset.border.bottom, asset.border.right, asset.border.top
    )
    .unwrap();
    writeln!(s, "  m_PixelsToUnits: {}", float(asset.pixels_to_units)).unwrap();
    writeln!(
        s,
        "  m_Pivot: {{x: {}, y: {}}}",
        float(asset.pivot.x),
        float(asset.pivot.y)
    )
    .unwrap();
    s.push_str("  m_Extrude: 0\n");
    s.push_str("  m_IsPolygon: 0\n");
    s.push_str("  m_PackingTag: \n"); // trailing space — verified in golden
    s.push_str("  m_RenderDataKey:\n");
    writeln!(s, "    {}: 21300000", guid_hex(&asset.own_guid)).unwrap();
    s.push_str("  m_AtlasTags: []\n");
    s.push_str("  m_SpriteAtlas: {fileID: 0}\n");

    // m_RD and m_AtlasRD are byte-identical for non-SpriteAtlas sprites
    // (verified across Orgel corpus). Guarded against SpriteAtlas use upstream.
    s.push_str("  m_RD:\n");
    write_render_data(&mut s, "    ", &asset.atlas_guid, &asset.render_data, asset.rect);
    s.push_str("  m_AtlasRD:\n");
    write_render_data(&mut s, "    ", &asset.atlas_guid, &asset.render_data, asset.rect);

    s.push_str("  m_PhysicsShape: []\n");
    s.push_str("  m_Bones: []\n");
    s.push_str("  m_ScriptableObjects: []\n");
    s.push_str("  m_SpriteID: \n"); // trailing space + final LF — verified

    s
}

fn write_rect_fields(s: &mut String, indent: &str, rect: Rect) {
    writeln!(s, "{indent}x: {}", rect.x).unwrap();
    writeln!(s, "{indent}y: {}", rect.y).unwrap();
    writeln!(s, "{indent}width: {}", rect.w).unwrap();
    writeln!(s, "{indent}height: {}", rect.h).unwrap();
}

fn write_render_data(
    s: &mut String,
    indent: &str,
    atlas_guid: &[u8; 16],
    rd: &RenderData,
    rect: Rect,
) {
    writeln!(s, "{indent}serializedVersion: 3").unwrap();
    writeln!(
        s,
        "{indent}texture: {{fileID: 2800000, guid: {}, type: 3}}",
        guid_hex(atlas_guid)
    )
    .unwrap();
    writeln!(s, "{indent}alphaTexture: {{fileID: 0}}").unwrap();
    writeln!(s, "{indent}secondaryTextures: []").unwrap();

    // m_SubMeshes — single submesh.
    writeln!(s, "{indent}m_SubMeshes:").unwrap();
    let inner = format!("{indent}  ");
    let inner = inner.as_str();
    writeln!(s, "{indent}- serializedVersion: 2").unwrap();
    writeln!(s, "{inner}firstByte: 0").unwrap();
    writeln!(s, "{inner}indexCount: {}", rd.index_count).unwrap();
    writeln!(s, "{inner}topology: 0").unwrap();
    writeln!(s, "{inner}baseVertex: 0").unwrap();
    writeln!(s, "{inner}firstVertex: 0").unwrap();
    writeln!(s, "{inner}vertexCount: {}", rd.vertex_count).unwrap();
    writeln!(s, "{inner}localAABB:").unwrap();
    writeln!(s, "{inner}  m_Center: {{x: 0, y: 0, z: 0}}").unwrap();
    writeln!(s, "{inner}  m_Extent: {{x: 0, y: 0, z: 0}}").unwrap();

    writeln!(s, "{indent}m_IndexBuffer: {}", rd.index_buffer_hex).unwrap();
    writeln!(s, "{indent}m_VertexData:").unwrap();
    writeln!(s, "{inner}serializedVersion: 3").unwrap();
    writeln!(s, "{inner}m_VertexCount: {}", rd.vertex_count).unwrap();
    writeln!(s, "{inner}m_Channels:").unwrap();
    // m_Channels lives inside m_VertexData; list dashes align with m_Channels
    // key indent (`inner`), not with m_VertexData's own indent.
    write_vertex_channels(s, inner);
    writeln!(s, "{inner}m_DataSize: {}", rd.data_size).unwrap();
    writeln!(s, "{inner}_typelessdata: {}", rd.typelessdata_hex).unwrap();

    writeln!(s, "{indent}m_Bindpose: []").unwrap();
    writeln!(s, "{indent}textureRect:").unwrap();
    writeln!(s, "{indent}  serializedVersion: 2").unwrap();
    write_rect_fields(s, &format!("{indent}  "), rect);
    writeln!(s, "{indent}textureRectOffset: {{x: 0, y: 0}}").unwrap();
    writeln!(s, "{indent}atlasRectOffset: {{x: -1, y: -1}}").unwrap(); // Unity default, NOT zero
    writeln!(s, "{indent}settingsRaw: 192").unwrap(); // hardcoded; panic-guarded if a future corpus diverges
    writeln!(
        s,
        "{indent}uvTransform: {{x: {}, y: {}, z: {}, w: {}}}",
        float(rd.uv_transform.x),
        float(rd.uv_transform.y),
        float(rd.uv_transform.z),
        float(rd.uv_transform.w)
    )
    .unwrap();
    writeln!(s, "{indent}downscaleMultiplier: 1").unwrap();
}

// 14 channel entries; only ch0 (position, dim 3, stream 0) and ch4 (UV, dim 2,
// stream 1) populated. Verified across Orgel corpus.
fn write_vertex_channels(s: &mut String, indent: &str) {
    let inner = format!("{indent}  ");
    let inner = inner.as_str();
    let entry = |stream: u8, dim: u8| {
        format!(
            "{indent}- stream: {stream}\n\
             {inner}offset: 0\n\
             {inner}format: 0\n\
             {inner}dimension: {dim}\n",
        )
    };
    s.push_str(&entry(0, 3)); // position
    for _ in 0..3 {
        s.push_str(&entry(0, 0));
    }
    s.push_str(&entry(1, 2)); // uv
    for _ in 0..9 {
        s.push_str(&entry(0, 0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_data;
    use crate::render_data::AtlasSize;
    use crate::tpsheet;

    const ORGEL: &str = include_str!("../tests/golden/orgel/Orgel.tpsheet");
    const CAKE_DECOLEFT_GOLDEN: &str =
        include_str!("../tests/golden/orgel/sprites/Cake__DecoLeft.asset");
    const CAKE_DECOLEFT_META: &str =
        include_str!("../tests/golden/orgel/sprites/Cake__DecoLeft.asset.meta");
    const ATLAS_META: &str = include_str!("../tests/golden/orgel/Orgel.png.meta");

    fn parse_guid_from_meta(meta: &str) -> [u8; 16] {
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

    #[test]
    fn cake_decoleft_full_byte_exact() {
        let sheet = tpsheet::parse(ORGEL).unwrap();
        let s = sheet
            .sprites
            .iter()
            .find(|s| s.name == "Cake__DecoLeft")
            .unwrap()
            .clone();
        let rd = render_data::build(
            s.rect,
            s.pivot,
            &s.geometry.vertices,
            &s.geometry.triangles,
            80.0,
            1.0,
            AtlasSize {
                width: 580,
                height: 580,
            },
        );
        let asset = SpriteAsset {
            name: s.name.clone(),
            rect: s.rect,
            border: s.border,
            pivot: s.pivot,
            pixels_to_units: 80.0,
            own_guid: parse_guid_from_meta(CAKE_DECOLEFT_META),
            atlas_guid: parse_guid_from_meta(ATLAS_META),
            render_data: rd,
        };
        let got = emit(&asset);
        if got != CAKE_DECOLEFT_GOLDEN {
            // Write both for easy diffing on failure.
            let _ = std::fs::create_dir_all("target/diff");
            let _ = std::fs::write("target/diff/Cake__DecoLeft.actual", &got);
            let _ = std::fs::write("target/diff/Cake__DecoLeft.expected", CAKE_DECOLEFT_GOLDEN);
            // Find first divergent offset.
            let g_bytes = got.as_bytes();
            let e_bytes = CAKE_DECOLEFT_GOLDEN.as_bytes();
            let mut first_diff = None;
            for (i, (a, b)) in g_bytes.iter().zip(e_bytes.iter()).enumerate() {
                if a != b {
                    first_diff = Some(i);
                    break;
                }
            }
            let i = first_diff.unwrap_or(g_bytes.len().min(e_bytes.len()));
            let lo = i.saturating_sub(16);
            let hi_g = (i + 16).min(g_bytes.len());
            let hi_e = (i + 16).min(e_bytes.len());
            panic!(
                "byte mismatch at offset {i} (got len={}, expected len={}):\n  got      ...{:?}...\n  expected ...{:?}...\n  diff files in target/diff/",
                g_bytes.len(),
                e_bytes.len(),
                String::from_utf8_lossy(&g_bytes[lo..hi_g]),
                String::from_utf8_lossy(&e_bytes[lo..hi_e]),
            );
        }
    }
}
