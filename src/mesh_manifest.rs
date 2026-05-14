// `.tps.mesh.json` schema + parser — sibling of `fab::Manifest` for SMA
// migrations. Sprite-emit and mesh-emit are kept structurally separate
// (the asset types are different — `!u!213` Sprite vs `!u!43` Mesh) so
// the schemas don't share a Combined enum.
//
// Schema (v1):
//
//   {
//     "version": 1,
//     "meshes": [
//       {
//         "fileId": -8704840387945618417,
//         "name": "frame_top",
//         "usedInCanvas": true,
//         "keepVertices": true,
//         "keepIndices": true,
//         "renderers": [
//           {
//             "sprite": "frame_top",            // tpsheet entry name
//             "flipX": false,
//             "flipY": false,
//             "drawMode": "simple",             // or "tiled"
//             "size": [1.0, 0.5],               // only for "tiled"
//             "localToRoot": [m00, m01, m02, m03, m10, m11, m12, m13]
//             //                                ^ 8-float row-major 2D affine
//           },
//           …
//         ]
//       },
//       …
//     ]
//   }
//
// `localToRoot` is the per-renderer 2D affine that combines flip ×
// renderer.localToWorld × root.worldToLocal — captured by the dumper
// from Unity directly so we don't re-derive a matrix chain at parse time.
// Stored as 8 floats so the JSON shape stays flat.
//
// Pipeline reads this sibling at `<tps_path>.mesh.json` (analogous to
// `<tps_path>.fab.json`). Absent → no mesh emit happens.

use serde::Deserialize;
use std::collections::HashSet;

#[derive(Debug, PartialEq)]
pub struct MeshManifest {
    pub meshes: Vec<MeshCombined>,
}

#[derive(Debug, PartialEq)]
pub struct MeshCombined {
    pub file_id: i64,
    pub name: String,
    pub used_in_canvas: bool,
    pub keep_vertices: bool,
    pub keep_indices: bool,
    pub renderers: Vec<MeshRenderer>,
}

#[derive(Debug, PartialEq)]
pub struct MeshRenderer {
    pub sprite: String,
    pub flip_x: bool,
    pub flip_y: bool,
    pub draw_mode: DrawMode,
    pub size: Option<[f32; 2]>,
    /// 8 floats, row-major: `[m00, m01, m02, m03, m10, m11, m12, m13]`.
    /// Combined flip × renderer.localToWorld × root.worldToLocal as
    /// captured by the dumper.
    pub local_to_root: [f32; 8],
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum DrawMode {
    Simple,
    Tiled,
}

#[derive(Debug)]
pub enum MeshManifestError {
    Json(serde_json::Error),
    UnsupportedVersion(u32),
    EmptyName,
    DuplicateName(String),
    DuplicateFileId(i64),
    EmptyRenderers(String),
    UnknownDrawMode(String),
    MissingSize(String),
    UnexpectedSize(String),
}

impl std::fmt::Display for MeshManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(e) => write!(f, "json: {e}"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported version: {v}"),
            Self::EmptyName => write!(f, "empty mesh name"),
            Self::DuplicateName(n) => write!(f, "duplicate mesh name: {n}"),
            Self::DuplicateFileId(id) => write!(f, "duplicate fileId: {id}"),
            Self::EmptyRenderers(n) => write!(f, "mesh '{n}' has empty renderers"),
            Self::UnknownDrawMode(m) => write!(f, "unknown drawMode: {m}"),
            Self::MissingSize(n) => write!(f, "mesh '{n}' tiled renderer missing size"),
            Self::UnexpectedSize(n) => write!(f, "mesh '{n}' simple renderer should not declare size"),
        }
    }
}

impl std::error::Error for MeshManifestError {}

pub fn parse(json: &str) -> Result<MeshManifest, MeshManifestError> {
    let raw: raw::Manifest = serde_json::from_str(json).map_err(MeshManifestError::Json)?;
    if raw.version != 1 {
        return Err(MeshManifestError::UnsupportedVersion(raw.version));
    }
    let mut names: HashSet<String> = HashSet::with_capacity(raw.meshes.len());
    let mut ids: HashSet<i64> = HashSet::with_capacity(raw.meshes.len());
    let mut out: Vec<MeshCombined> = Vec::with_capacity(raw.meshes.len());
    for m in raw.meshes {
        if m.name.is_empty() {
            return Err(MeshManifestError::EmptyName);
        }
        if !names.insert(m.name.clone()) {
            return Err(MeshManifestError::DuplicateName(m.name));
        }
        if !ids.insert(m.file_id) {
            return Err(MeshManifestError::DuplicateFileId(m.file_id));
        }
        if m.renderers.is_empty() {
            return Err(MeshManifestError::EmptyRenderers(m.name));
        }
        let mut renderers: Vec<MeshRenderer> = Vec::with_capacity(m.renderers.len());
        for r in m.renderers {
            let draw_mode = match r.draw_mode.as_str() {
                "simple" => DrawMode::Simple,
                "tiled" => DrawMode::Tiled,
                other => return Err(MeshManifestError::UnknownDrawMode(other.to_string())),
            };
            match (draw_mode, r.size.is_some()) {
                (DrawMode::Tiled, false) => {
                    return Err(MeshManifestError::MissingSize(m.name.clone()))
                }
                (DrawMode::Simple, true) => {
                    return Err(MeshManifestError::UnexpectedSize(m.name.clone()))
                }
                _ => {}
            }
            renderers.push(MeshRenderer {
                sprite: r.sprite,
                flip_x: r.flip_x.unwrap_or(false),
                flip_y: r.flip_y.unwrap_or(false),
                draw_mode,
                size: r.size,
                local_to_root: r.local_to_root,
            });
        }
        out.push(MeshCombined {
            file_id: m.file_id,
            name: m.name,
            used_in_canvas: m.used_in_canvas,
            keep_vertices: m.keep_vertices.unwrap_or(true),
            keep_indices: m.keep_indices.unwrap_or(true),
            renderers,
        });
    }
    Ok(MeshManifest { meshes: out })
}

mod raw {
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Manifest {
        pub version: u32,
        #[serde(default)]
        pub meshes: Vec<MeshCombined>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub struct MeshCombined {
        pub file_id: i64,
        pub name: String,
        pub used_in_canvas: bool,
        pub keep_vertices: Option<bool>,
        pub keep_indices: Option<bool>,
        #[serde(default)]
        pub renderers: Vec<MeshRenderer>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub struct MeshRenderer {
        pub sprite: String,
        pub flip_x: Option<bool>,
        pub flip_y: Option<bool>,
        pub draw_mode: String,
        pub size: Option<[f32; 2]>,
        pub local_to_root: [f32; 8],
    }
}

// Forces serde to keep the import alive even though only `mod raw` uses it.
#[allow(dead_code)]
fn _serde_anchor() -> Option<impl Deserialize<'static>> {
    let v: Option<u32> = None;
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_single_mesh() {
        let m = parse(
            r#"{
              "version": 1,
              "meshes": [{
                "fileId": 12345,
                "name": "frame_top",
                "usedInCanvas": true,
                "renderers": [{
                  "sprite": "frame_top_strip",
                  "drawMode": "simple",
                  "localToRoot": [1, 0, 0, 0, 0, 1, 0, 0]
                }]
              }]
            }"#,
        )
        .unwrap();
        assert_eq!(m.meshes.len(), 1);
        let c = &m.meshes[0];
        assert_eq!(c.file_id, 12345);
        assert_eq!(c.name, "frame_top");
        assert!(c.used_in_canvas);
        assert!(c.keep_vertices);
        assert!(c.keep_indices);
        assert_eq!(c.renderers.len(), 1);
        assert_eq!(c.renderers[0].draw_mode, DrawMode::Simple);
        assert!(!c.renderers[0].flip_x);
        assert!(c.renderers[0].size.is_none());
    }

    #[test]
    fn parse_tiled_renderer_with_size() {
        let m = parse(
            r#"{
              "version": 1,
              "meshes": [{
                "fileId": -9000,
                "name": "wall_line",
                "usedInCanvas": false,
                "keepVertices": false,
                "keepIndices": false,
                "renderers": [{
                  "sprite": "wall_brick",
                  "flipX": true,
                  "drawMode": "tiled",
                  "size": [4.05, 1.0],
                  "localToRoot": [1, 0, 0, 0, 0, 1, 0, 0]
                }]
              }]
            }"#,
        )
        .unwrap();
        let c = &m.meshes[0];
        assert!(!c.used_in_canvas);
        assert!(!c.keep_vertices);
        assert_eq!(c.renderers[0].draw_mode, DrawMode::Tiled);
        assert_eq!(c.renderers[0].size, Some([4.05, 1.0]));
        assert!(c.renderers[0].flip_x);
    }

    #[test]
    fn parse_rejects_unsupported_version() {
        assert!(matches!(
            parse(r#"{ "version": 2, "meshes": [] }"#),
            Err(MeshManifestError::UnsupportedVersion(2))
        ));
    }

    #[test]
    fn parse_rejects_duplicate_name() {
        let m = parse(
            r#"{
              "version": 1,
              "meshes": [
                {"fileId": 1, "name": "x", "usedInCanvas": true,
                 "renderers": [{"sprite": "a", "drawMode": "simple",
                                "localToRoot": [1,0,0,0,0,1,0,0]}]},
                {"fileId": 2, "name": "x", "usedInCanvas": true,
                 "renderers": [{"sprite": "b", "drawMode": "simple",
                                "localToRoot": [1,0,0,0,0,1,0,0]}]}
              ]
            }"#,
        );
        assert!(matches!(m, Err(MeshManifestError::DuplicateName(n)) if n == "x"));
    }

    #[test]
    fn parse_rejects_duplicate_file_id() {
        let m = parse(
            r#"{
              "version": 1,
              "meshes": [
                {"fileId": 1, "name": "a", "usedInCanvas": true,
                 "renderers": [{"sprite": "a", "drawMode": "simple",
                                "localToRoot": [1,0,0,0,0,1,0,0]}]},
                {"fileId": 1, "name": "b", "usedInCanvas": true,
                 "renderers": [{"sprite": "b", "drawMode": "simple",
                                "localToRoot": [1,0,0,0,0,1,0,0]}]}
              ]
            }"#,
        );
        assert!(matches!(m, Err(MeshManifestError::DuplicateFileId(1))));
    }

    #[test]
    fn parse_rejects_tiled_without_size() {
        let m = parse(
            r#"{ "version": 1, "meshes": [{
              "fileId": 1, "name": "x", "usedInCanvas": true,
              "renderers": [{"sprite": "a", "drawMode": "tiled",
                             "localToRoot": [1,0,0,0,0,1,0,0]}]
            }]}"#,
        );
        assert!(matches!(m, Err(MeshManifestError::MissingSize(_))));
    }

    #[test]
    fn parse_rejects_simple_with_size() {
        let m = parse(
            r#"{ "version": 1, "meshes": [{
              "fileId": 1, "name": "x", "usedInCanvas": true,
              "renderers": [{"sprite": "a", "drawMode": "simple", "size": [1, 1],
                             "localToRoot": [1,0,0,0,0,1,0,0]}]
            }]}"#,
        );
        assert!(matches!(m, Err(MeshManifestError::UnexpectedSize(_))));
    }

    #[test]
    fn parse_rejects_unknown_draw_mode() {
        let m = parse(
            r#"{ "version": 1, "meshes": [{
              "fileId": 1, "name": "x", "usedInCanvas": true,
              "renderers": [{"sprite": "a", "drawMode": "stretched",
                             "localToRoot": [1,0,0,0,0,1,0,0]}]
            }]}"#,
        );
        assert!(matches!(m, Err(MeshManifestError::UnknownDrawMode(s)) if s == "stretched"));
    }

    #[test]
    fn parse_rejects_empty_renderers() {
        let m = parse(
            r#"{ "version": 1, "meshes": [{
              "fileId": 1, "name": "x", "usedInCanvas": true,
              "renderers": []
            }]}"#,
        );
        assert!(matches!(m, Err(MeshManifestError::EmptyRenderers(n)) if n == "x"));
    }

    #[test]
    fn parse_rejects_unknown_top_level_field() {
        // `deny_unknown_fields` on raw::Manifest catches typos.
        let m = parse(r#"{ "version": 1, "meshes": [], "extra": 0 }"#);
        assert!(matches!(m, Err(MeshManifestError::Json(_))));
    }
}
