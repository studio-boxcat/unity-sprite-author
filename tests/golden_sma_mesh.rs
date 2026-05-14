// Phase 2b TDD oracle for `unity_sprite_author::mesh_emit`.
//
// `box_29_ghost_first_mesh_byte_exact` (default-run): construct a MeshAsset
// from data decoded out of the committed fixture's first Mesh sub-asset
// (`tests/golden/sma/box_29_ghost/Box_29_Ghost.asset`, fileID
// -8704840387945618417), re-emit via `mesh_emit::emit_mesh_body_canvas`,
// and assert byte-exact equality against the extracted sub-asset bytes.
// Closes the YAML-template / VBO-encoding loop for the CanvasRenderer
// layout.
//
// `box_29_ghost_roundtrip` (`#[ignore]`): full multi-mesh round-trip of
// all 32 sub-assets in the fixture — pinned until
// `emit_mesh_asset` (multi-mesh assembly) and the SpriteRenderer
// layout land.

use std::fs;
use std::path::Path;

use unity_sprite_author::mesh_emit::{self, MeshAsset};

/// First Mesh sub-asset (`!u!43 &-8704840387945618417`) extracted byte-by-byte
/// from `Box_29_Ghost.asset` lines 4..180 (`Mesh:` through the trailing
/// `m_IndexCount: 0` of m_MeshLodInfo, inclusive of the final newline).
fn extract_first_mesh_body() -> String {
    let asset = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden/sma/box_29_ghost/Box_29_Ghost.asset"),
    )
    .expect("read fixture");
    let lines: Vec<&str> = asset.lines().collect();
    let start = lines
        .iter()
        .position(|l| l.starts_with("Mesh:"))
        .expect("Mesh: header");
    // Stop at the next `--- !u!` divider.
    let end_rel = lines[start + 1..]
        .iter()
        .position(|l| l.starts_with("--- !u!"))
        .expect("next sub-asset divider");
    let end = start + 1 + end_rel;
    let mut out = lines[start..end].join("\n");
    out.push('\n');
    out
}

#[test]
fn box_29_ghost_first_mesh_byte_exact() {
    let golden = extract_first_mesh_body();

    // The fixture is the first Mesh sub-asset of Box_29_Ghost: 28 verts,
    // 72 indices, CanvasRenderer layout (color Color32 = ffffffff per
    // vertex), AABB matches the YAML literal.
    let m = decode_mesh_asset_from_golden(&golden);
    let got = mesh_emit::emit_mesh_body_canvas(&m);

    if got == golden {
        return;
    }

    // Diff dump: first divergent byte + a window.
    let off = got
        .bytes()
        .zip(golden.bytes())
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| got.len().min(golden.len()));
    let lo = off.saturating_sub(40);
    let hi_g = (off + 80).min(got.len());
    let hi_e = (off + 80).min(golden.len());
    let _ = fs::create_dir_all("target/diff");
    let _ = fs::write("target/diff/box_29_ghost_first_mesh.actual", &got);
    let _ = fs::write("target/diff/box_29_ghost_first_mesh.expected", &golden);
    panic!(
        "byte mismatch at offset {off} (got len={}, golden len={})\n\
         got     [{lo}..{hi_g}]: {:?}\n\
         golden  [{lo}..{hi_e}]: {:?}",
        got.len(),
        golden.len(),
        &got[lo..hi_g],
        &golden[lo..hi_e],
    );
}

/// Decode the variable bits (verts/colors/uvs/indices/AABB) directly out
/// of the golden YAML. This is test scaffolding — no production parser
/// for the Mesh asset exists yet.
fn decode_mesh_asset_from_golden(yaml: &str) -> MeshAsset {
    // index buffer
    let ib_hex = find_value(yaml, "m_IndexBuffer:");
    let indices: Vec<u16> = ib_hex
        .as_bytes()
        .chunks(4)
        .map(|c| {
            let lo = u8::from_str_radix(std::str::from_utf8(&c[0..2]).unwrap(), 16).unwrap();
            let hi = u8::from_str_radix(std::str::from_utf8(&c[2..4]).unwrap(), 16).unwrap();
            (lo as u16) | ((hi as u16) << 8)
        })
        .collect();

    // Detect layout from `m_IsReadable` (1 → CanvasRenderer, 0 → SpriteRenderer).
    let is_readable = parse_int_field(yaml, "  m_IsReadable:") != 0;
    let used_in_canvas = is_readable;

    // typeless data: stride depends on layout.
    //   CanvasRenderer: 24 (pos f32x3 + color Color32 + uv f32x2)
    //   SpriteRenderer: 16 (pos f32x3 + uv half2)
    let td_hex = find_value(yaml, "_typelessdata:");
    let td_bytes: Vec<u8> = (0..td_hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&td_hex[i..i + 2], 16).unwrap())
        .collect();
    let stride = if used_in_canvas { 24 } else { 16 };
    let vc = td_bytes.len() / stride;
    let mut vertices = Vec::with_capacity(vc);
    let mut colors = Vec::with_capacity(vc);
    let mut uvs = Vec::with_capacity(vc);
    let read_f32 = |bs: &[u8]| -> f32 {
        f32::from_le_bytes([bs[0], bs[1], bs[2], bs[3]])
    };
    for i in 0..vc {
        let o = i * stride;
        let px = read_f32(&td_bytes[o..o + 4]);
        let py = read_f32(&td_bytes[o + 4..o + 8]);
        let pz = read_f32(&td_bytes[o + 8..o + 12]);
        vertices.push([px, py, pz]);
        if used_in_canvas {
            colors.push([
                td_bytes[o + 12],
                td_bytes[o + 13],
                td_bytes[o + 14],
                td_bytes[o + 15],
            ]);
            let ux = read_f32(&td_bytes[o + 16..o + 20]);
            let uy = read_f32(&td_bytes[o + 20..o + 24]);
            uvs.push([ux, uy]);
        } else {
            // half2 UV → up-convert to f32 via reference table for the test.
            // We don't have float_from_half here; the round-trip emits via
            // float_to_half, so we feed back the bits as the f32 that maps
            // to those bits. Practical: parse u16 → reinterpret as the
            // exact f32 reproducing the original half via mesh_emit's
            // float_to_half_inverse helper. For golden round-trip we just
            // need a value that re-encodes to the same u16.
            let u_h = u16::from_le_bytes([td_bytes[o + 12], td_bytes[o + 13]]);
            let v_h = u16::from_le_bytes([td_bytes[o + 14], td_bytes[o + 15]]);
            uvs.push([half_to_f32(u_h), half_to_f32(v_h)]);
        }
    }

    let (cx, cy, cz) = parse_xyz(yaml, "  m_LocalAABB:", "    m_Center:");
    let (ex, ey, ez) = parse_xyz(yaml, "  m_LocalAABB:", "    m_Extent:");

    let keep_vertices = parse_int_field(yaml, "  m_KeepVertices:") != 0;
    let keep_indices = parse_int_field(yaml, "  m_KeepIndices:") != 0;
    MeshAsset {
        file_id: 0,
        name: String::new(),
        vertices,
        uvs,
        colors,
        indices,
        used_in_canvas,
        keep_vertices,
        keep_indices,
        aabb_center: [cx, cy, cz],
        aabb_extent: [ex, ey, ez],
    }
}

/// IEEE 754 binary16 → f32. Standard reference impl; only used in tests
/// to round-trip the half2 UV bytes from the golden into an f32 input
/// that `mesh_emit::float_to_half` will re-encode to the same u16.
fn half_to_f32(h: u16) -> f32 {
    let sign = ((h & 0x8000) as u32) << 16;
    let exp = ((h >> 10) & 0x1F) as u32;
    let mant = (h & 0x3FF) as u32;
    if exp == 0 {
        if mant == 0 {
            return f32::from_bits(sign);
        }
        // Subnormal.
        let mut m = mant;
        let mut e = 1u32;
        while (m & 0x400) == 0 {
            m <<= 1;
            e += 1;
        }
        let exp32 = 127 - 15 - e + 1;
        return f32::from_bits(sign | (exp32 << 23) | ((m & 0x3FF) << 13));
    }
    if exp == 0x1F {
        if mant == 0 {
            return f32::from_bits(sign | 0x7f80_0000); // Inf
        }
        return f32::from_bits(sign | 0x7fc0_0000 | (mant << 13)); // NaN
    }
    let exp32 = exp + (127 - 15);
    f32::from_bits(sign | (exp32 << 23) | (mant << 13))
}

/// Drop every sub-asset block that isn't a Mesh (`!u!43`). Keeps the
/// leading two-line YAML preamble.
fn strip_non_mesh_sub_assets(yaml: &str) -> String {
    let mut out = String::with_capacity(yaml.len());
    let mut keep = true; // preamble is kept
    for line in yaml.lines() {
        if let Some(after) = line.strip_prefix("--- !u!") {
            keep = after.starts_with("43 ");
        }
        if keep {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn parse_int_field(yaml: &str, key: &str) -> i32 {
    let line = yaml.lines().find(|l| l.starts_with(key)).expect("key missing");
    line.split_once(':').unwrap().1.trim().parse().unwrap()
}

fn find_value(yaml: &str, key: &str) -> String {
    let line = yaml.lines().find(|l| l.contains(key)).expect("key missing");
    let val = line.split_once(key).unwrap().1.trim();
    val.to_string()
}

fn parse_xyz(yaml: &str, section: &str, key: &str) -> (f32, f32, f32) {
    let mut it = yaml.lines().skip_while(|l| !l.starts_with(section));
    it.next(); // consume the section header
    let line = it.find(|l| l.trim_start().starts_with(key.trim_start())).expect("xyz key");
    // line e.g. "    m_Center: {x: 0, y: 0.32053053, z: 0}"
    let inner = line.split('{').nth(1).unwrap().trim_end_matches('}');
    let parts: Vec<&str> = inner.split(',').collect();
    let parse = |s: &str| -> f32 {
        s.split(':').nth(1).unwrap().trim().parse().unwrap()
    };
    (parse(parts[0]), parse(parts[1]), parse(parts[2]))
}

#[test]
fn box_29_ghost_roundtrip() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/sma/box_29_ghost/Box_29_Ghost.asset");
    let golden = fs::read_to_string(&path).expect("read fixture");

    // The committed fixture mixes 32 Mesh sub-assets (`!u!43`) with 3 Odin
    // metadata MonoBehaviours (`!u!114`) — the latter are out of scope for
    // SMA mesh emit. Strip them from the golden before comparing and skip
    // them while collecting MeshAssets.
    let golden = strip_non_mesh_sub_assets(&golden);
    let lines: Vec<&str> = golden.lines().collect();
    let header_indices: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| l.starts_with("--- !u!43 &").then_some(i))
        .collect();
    assert!(!header_indices.is_empty(), "no sub-asset headers");
    let mut meshes = Vec::with_capacity(header_indices.len());
    for w in 0..header_indices.len() {
        let start = header_indices[w];
        let end = header_indices.get(w + 1).copied().unwrap_or(lines.len());
        let file_id: i64 = lines[start]
            .trim_start_matches("--- !u!43 &")
            .parse()
            .expect("parse file_id");
        let body_yaml = lines[start + 1..end].join("\n") + "\n";
        let mut m = decode_mesh_asset_from_golden(&body_yaml);
        m.file_id = file_id;
        meshes.push(m);
    }

    let got = mesh_emit::emit_mesh_asset(&meshes);
    let got_str = String::from_utf8(got).expect("utf8");
    if got_str == golden {
        return;
    }

    let off = got_str
        .bytes()
        .zip(golden.bytes())
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| got_str.len().min(golden.len()));
    let lo = off.saturating_sub(40);
    let hi_g = (off + 80).min(got_str.len());
    let hi_e = (off + 80).min(golden.len());
    let _ = fs::create_dir_all("target/diff");
    let _ = fs::write("target/diff/box_29_ghost_roundtrip.actual", &got_str);
    let _ = fs::write("target/diff/box_29_ghost_roundtrip.expected", &golden);
    panic!(
        "byte mismatch at offset {off} (got len={}, golden len={})\n\
         got     [{lo}..{hi_g}]: {:?}\n\
         golden  [{lo}..{hi_e}]: {:?}",
        got_str.len(),
        golden.len(),
        &got_str[lo..hi_g],
        &golden[lo..hi_e],
    );
}
