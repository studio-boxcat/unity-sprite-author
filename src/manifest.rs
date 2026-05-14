// Unified `.tps.fab.json` / `.tps.mesh.json` manifest — v3 schema.
//
// Replaces the two flat-array schemas (`fab::Manifest` + `mesh_manifest::MeshManifest`)
// with a single tree-shaped schema mirroring pspec (`tools/pspec` in meow-tower).
// Each `Tree` is one authored output (a CSA-published Sprite or an SMA-published
// Mesh); children are GameObjects whose transforms compose down the tree.
//
// > **Related:** [[fab.md]], [[sma-migration.md]], pspec orientation
//
// JSON shape:
//
//   {
//     "version": 1,
//     "trees": [
//       {
//         "name": "Silloutte1",
//         "scale": 0.01,                       // root's `scaleFactor` (CSA) or 1.0 (SMA)
//         "rootAnchored": [141.8, 370.875],    // optional, only matters for FMA residue
//         "output": "csa",                      // or { "type":"sma", "fileId":…, "outputPath":…, "usedInCanvas":… }
//         "children": [
//           {
//             "name": "Image",
//             "pos": [0, -22.25],
//             "sizeDelta": [212.5, 545],
//             "pivot": [0.5, 0.5],
//             "graphic": { "type":"polygon", "color":"32264DBD", "vertices":[[…]] }
//           },
//           {
//             "name": "B",
//             "pos": [0, -294.75],
//             "sizeDelta": [212.5, 17.5],
//             "pivot": [0.5, 1],
//             "graphic": { "type":"sprite", "sprite":"Mansion_…__B", "method":"MX" }
//           },
//           …
//         ]
//       }
//     ]
//   }
//
// Defaults per node:
//   pos    = [0, 0]
//   sizeDelta = [0, 0]   (only matters for size-fitted methods; SMA leaves it 0)
//   pivot  = [0.5, 0.5]
//   scale  = 1.0         (uniform; per-axis `[x, y]` for X/Y flip)
//   children = []
//   graphic = none (interior nodes carry only a transform)
//
// Defaults per graphic:
//   sprite:  method = "ID", uiScale = 100, drawMode = "simple"
//   polygon: triangles = ear-clip
//   sprite-renderer (SMA): drawMode = "simple", flipX/flipY = false

#![allow(dead_code)]

use serde::Deserialize;
use std::collections::HashSet;
use std::fmt;

#[derive(Debug, PartialEq)]
pub struct Manifest {
    pub trees: Vec<Tree>,
}

#[derive(Debug, PartialEq)]
pub struct Tree {
    pub name: String,
    /// CSA `_scaleFactor` (0.01 typical) or SMA world-unit (1.0 typical).
    pub scale: f32,
    /// CSA root's `RectTransform.anchoredPosition`. Defaults to (0, 0). Only
    /// matters for FMA-residue reproduction when non-origin.
    pub root_anchored: [f32; 2],
    pub output: Output,
    pub root: Node,
}

#[derive(Debug, PartialEq)]
pub enum Output {
    /// CSA-published Sprite. Output path / GUID are derived from the
    /// `_sprite` reference at migration time, not declared here.
    Csa,
    /// SMA-published Mesh. Output path + Mesh sub-asset fileId + canvas-vs-
    /// sprite-renderer layout declared explicitly.
    Sma {
        file_id: i64,
        output_path: String,
        used_in_canvas: bool,
        keep_vertices: bool,
        keep_indices: bool,
    },
}

#[derive(Debug, PartialEq)]
pub struct Node {
    pub name: String,
    pub pos: [f32; 2],
    pub size_delta: [f32; 2],
    pub pivot: [f32; 2],
    /// Per-axis uniform/non-uniform scale. `[1, 1]` = identity.
    pub scale: [f32; 2],
    pub rot_deg: f32,
    pub graphic: Option<Graphic>,
    pub children: Vec<Node>,
}

#[derive(Debug, PartialEq)]
pub enum Graphic {
    /// UIIcon / UISlice (CSA hierarchy) or atlas-sprite under SMA.
    Sprite {
        sprite: String,
        method: SpriteMethod,
        ui_scale: f32,
        border_mult: f32,
        flip_x: bool,
        flip_y: bool,
    },
    /// UISolid or color-quad polygon.
    Polygon {
        /// Resolved tpsheet entry name (e.g. `Color_32264DBDFF` for color `"32264DBD"`,
        /// `Color_32264DFF` for `"32264D"`).
        polygon_sprite: String,
        vertices: Vec<[f32; 2]>,
        triangles: Option<Vec<u16>>,
    },
    /// SMA SpriteRenderer leaf — different VBO layout, tile mode option.
    SpriteRenderer {
        sprite: String,
        draw_mode: DrawMode,
        size: Option<[f32; 2]>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpriteMethod {
    Id,
    Mx, My, Mxy,
    Tx, Ty, TxMc3,
    R1c3, R3c3, R3c3Nf,
    MxR1c3, MxR1c4, MxR3c2, MxR3c3, MxR3c4, MxR3c6,
    MyR2c2, MyR2c3, MyR3c1, MyR3c2, MyR3c3,
    MxyR3c3, MxyR3c3Nf,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DrawMode {
    Simple,
    Tiled,
}

#[derive(Debug)]
pub enum ManifestError {
    Json(serde_json::Error),
    UnsupportedVersion(u32),
    EmptyName,
    DuplicateName(String),
    BadColor { tree: String, color: String },
    UnknownMethod { tree: String, method: String },
    UnknownDrawMode { tree: String, mode: String },
    PolygonTooFewVertices { tree: String, n: usize },
    GraphicShape { tree: String, reason: &'static str },
    OutputShape { tree: String, reason: &'static str },
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(e) => write!(f, "json: {e}"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported version: {v}"),
            Self::EmptyName => write!(f, "tree.name must be non-empty"),
            Self::DuplicateName(n) => write!(f, "duplicate tree.name: {n:?}"),
            Self::BadColor { tree, color } => write!(
                f,
                "tree {tree:?} polygon color {color:?}: expected 6 or 8 hex chars"
            ),
            Self::UnknownMethod { tree, method } => {
                write!(f, "tree {tree:?}: unknown sprite method {method:?}")
            }
            Self::UnknownDrawMode { tree, mode } => {
                write!(f, "tree {tree:?}: unknown drawMode {mode:?}")
            }
            Self::PolygonTooFewVertices { tree, n } => write!(
                f,
                "tree {tree:?} polygon: need ≥ 3 vertices, got {n}"
            ),
            Self::GraphicShape { tree, reason } => {
                write!(f, "tree {tree:?} graphic: {reason}")
            }
            Self::OutputShape { tree, reason } => {
                write!(f, "tree {tree:?} output: {reason}")
            }
        }
    }
}

impl std::error::Error for ManifestError {}

pub fn parse(json: &str) -> Result<Manifest, ManifestError> {
    let raw: raw::Manifest = serde_json::from_str(json).map_err(ManifestError::Json)?;
    if raw.version != 1 {
        return Err(ManifestError::UnsupportedVersion(raw.version));
    }
    let mut seen: HashSet<String> = HashSet::with_capacity(raw.trees.len());
    let mut trees: Vec<Tree> = Vec::with_capacity(raw.trees.len());
    for t in raw.trees {
        if t.name.is_empty() {
            return Err(ManifestError::EmptyName);
        }
        if !seen.insert(t.name.clone()) {
            return Err(ManifestError::DuplicateName(t.name));
        }
        let output = translate_output(&t.name, t.output)?;
        let root = translate_node(&t.name, t.root)?;
        trees.push(Tree {
            name: t.name,
            scale: t.scale.unwrap_or(1.0),
            root_anchored: t.root_anchored.unwrap_or([0.0, 0.0]),
            output,
            root,
        });
    }
    Ok(Manifest { trees })
}

fn translate_output(tree: &str, raw: raw::OutputSpec) -> Result<Output, ManifestError> {
    match raw {
        raw::OutputSpec::Tag(s) if s == "csa" => Ok(Output::Csa),
        raw::OutputSpec::Tag(s) => Err(ManifestError::OutputShape {
            tree: tree.to_string(),
            reason: if s == "sma" {
                "sma output must be an object with fileId/outputPath/usedInCanvas"
            } else {
                "unknown output tag"
            },
        }),
        raw::OutputSpec::Object {
            kind,
            file_id,
            output_path,
            used_in_canvas,
            keep_vertices,
            keep_indices,
        } => {
            if kind != "sma" {
                return Err(ManifestError::OutputShape {
                    tree: tree.to_string(),
                    reason: "object output must declare type: \"sma\"",
                });
            }
            let file_id = file_id.ok_or(ManifestError::OutputShape {
                tree: tree.to_string(),
                reason: "sma output requires `fileId`",
            })?;
            let output_path = output_path.ok_or(ManifestError::OutputShape {
                tree: tree.to_string(),
                reason: "sma output requires `outputPath`",
            })?;
            let used_in_canvas = used_in_canvas.ok_or(ManifestError::OutputShape {
                tree: tree.to_string(),
                reason: "sma output requires `usedInCanvas`",
            })?;
            Ok(Output::Sma {
                file_id,
                output_path,
                used_in_canvas,
                keep_vertices: keep_vertices.unwrap_or(true),
                keep_indices: keep_indices.unwrap_or(true),
            })
        }
    }
}

fn translate_node(tree: &str, raw: raw::Node) -> Result<Node, ManifestError> {
    let scale = raw.scale.as_ref().map(raw::ScaleSpec::resolve).unwrap_or([1.0, 1.0]);
    let graphic = match raw.graphic {
        Some(g) => Some(translate_graphic(tree, g)?),
        None => None,
    };
    let mut children = Vec::with_capacity(raw.children.len());
    for c in raw.children {
        children.push(translate_node(tree, c)?);
    }
    Ok(Node {
        name: raw.name.unwrap_or_default(),
        pos: raw.pos.unwrap_or([0.0, 0.0]),
        size_delta: raw.size_delta.unwrap_or([0.0, 0.0]),
        pivot: raw.pivot.unwrap_or([0.5, 0.5]),
        scale,
        rot_deg: raw.rot_deg.unwrap_or(0.0),
        graphic,
        children,
    })
}

fn translate_graphic(tree: &str, g: raw::Graphic) -> Result<Graphic, ManifestError> {
    match g.kind.as_str() {
        "sprite" => {
            let sprite = g.sprite.ok_or(ManifestError::GraphicShape {
                tree: tree.to_string(),
                reason: "sprite graphic requires `sprite`",
            })?;
            let method_str = g.method.as_deref().unwrap_or("ID");
            let method = parse_method(method_str).ok_or_else(|| ManifestError::UnknownMethod {
                tree: tree.to_string(),
                method: method_str.to_string(),
            })?;
            Ok(Graphic::Sprite {
                sprite,
                method,
                ui_scale: g.ui_scale.unwrap_or(100.0),
                border_mult: g.border_mult.unwrap_or(1.0),
                flip_x: g.flip_x.unwrap_or(false),
                flip_y: g.flip_y.unwrap_or(false),
            })
        }
        "polygon" => {
            let color = g.color.ok_or(ManifestError::GraphicShape {
                tree: tree.to_string(),
                reason: "polygon graphic requires `color`",
            })?;
            let polygon_sprite = resolve_color(tree, &color)?;
            let vertices = g.vertices.unwrap_or_default();
            if vertices.len() < 3 {
                return Err(ManifestError::PolygonTooFewVertices {
                    tree: tree.to_string(),
                    n: vertices.len(),
                });
            }
            Ok(Graphic::Polygon {
                polygon_sprite,
                vertices,
                triangles: g.triangles,
            })
        }
        "sprite-renderer" => {
            let sprite = g.sprite.ok_or(ManifestError::GraphicShape {
                tree: tree.to_string(),
                reason: "sprite-renderer graphic requires `sprite`",
            })?;
            let draw_mode = match g.draw_mode.as_deref().unwrap_or("simple") {
                "simple" => DrawMode::Simple,
                "tiled" => DrawMode::Tiled,
                other => {
                    return Err(ManifestError::UnknownDrawMode {
                        tree: tree.to_string(),
                        mode: other.to_string(),
                    })
                }
            };
            Ok(Graphic::SpriteRenderer {
                sprite,
                draw_mode,
                size: g.size,
            })
        }
        other => Err(ManifestError::GraphicShape {
            tree: tree.to_string(),
            reason: match other {
                "sprite" | "polygon" | "sprite-renderer" => "internal",
                _ => "type must be \"sprite\" | \"polygon\" | \"sprite-renderer\"",
            },
        }),
    }
}

fn resolve_color(tree: &str, c: &str) -> Result<String, ManifestError> {
    let len = c.len();
    let valid = (len == 6 || len == 8) && c.bytes().all(|b| b.is_ascii_hexdigit());
    if !valid {
        return Err(ManifestError::BadColor {
            tree: tree.to_string(),
            color: c.to_string(),
        });
    }
    let upper = c.to_ascii_uppercase();
    Ok(if len == 6 {
        format!("Color_{upper}FF")
    } else {
        format!("Color_{upper}")
    })
}

fn parse_method(s: &str) -> Option<SpriteMethod> {
    Some(match s {
        "ID" => SpriteMethod::Id,
        "MX" => SpriteMethod::Mx,
        "MY" => SpriteMethod::My,
        "MXY" => SpriteMethod::Mxy,
        "TX" => SpriteMethod::Tx,
        "TY" => SpriteMethod::Ty,
        "TX_MC3" => SpriteMethod::TxMc3,
        "R1C3" => SpriteMethod::R1c3,
        "R3C3" => SpriteMethod::R3c3,
        "R3C3_NF" => SpriteMethod::R3c3Nf,
        "MX_R1C3" => SpriteMethod::MxR1c3,
        "MX_R1C4" => SpriteMethod::MxR1c4,
        "MX_R3C2" => SpriteMethod::MxR3c2,
        "MX_R3C3" => SpriteMethod::MxR3c3,
        "MX_R3C4" => SpriteMethod::MxR3c4,
        "MX_R3C6" => SpriteMethod::MxR3c6,
        "MY_R2C2" => SpriteMethod::MyR2c2,
        "MY_R2C3" => SpriteMethod::MyR2c3,
        "MY_R3C1" => SpriteMethod::MyR3c1,
        "MY_R3C2" => SpriteMethod::MyR3c2,
        "MY_R3C3" => SpriteMethod::MyR3c3,
        "MXY_R3C3" => SpriteMethod::MxyR3c3,
        "MXY_R3C3_NF" => SpriteMethod::MxyR3c3Nf,
        _ => return None,
    })
}

mod raw {
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Manifest {
        pub version: u32,
        #[serde(default)]
        pub trees: Vec<Tree>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub struct Tree {
        pub name: String,
        pub scale: Option<f32>,
        pub root_anchored: Option<[f32; 2]>,
        pub output: OutputSpec,
        #[serde(default = "default_root")]
        pub root: Node,
    }

    fn default_root() -> Node {
        Node {
            name: None, pos: None, size_delta: None, pivot: None,
            scale: None, rot_deg: None, graphic: None, children: Vec::new(),
        }
    }

    #[derive(Deserialize)]
    #[serde(untagged)]
    pub enum OutputSpec {
        Tag(String),
        Object {
            #[serde(rename = "type")]
            kind: String,
            #[serde(rename = "fileId")]
            file_id: Option<i64>,
            #[serde(rename = "outputPath")]
            output_path: Option<String>,
            #[serde(rename = "usedInCanvas")]
            used_in_canvas: Option<bool>,
            #[serde(rename = "keepVertices")]
            keep_vertices: Option<bool>,
            #[serde(rename = "keepIndices")]
            keep_indices: Option<bool>,
        },
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub struct Node {
        pub name: Option<String>,
        pub pos: Option<[f32; 2]>,
        pub size_delta: Option<[f32; 2]>,
        pub pivot: Option<[f32; 2]>,
        pub scale: Option<ScaleSpec>,
        pub rot_deg: Option<f32>,
        pub graphic: Option<Graphic>,
        #[serde(default)]
        pub children: Vec<Node>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub struct Graphic {
        #[serde(rename = "type")]
        pub kind: String,

        // sprite + sprite-renderer
        pub sprite: Option<String>,
        // sprite only
        pub method: Option<String>,
        pub ui_scale: Option<f32>,
        pub border_mult: Option<f32>,
        pub flip_x: Option<bool>,
        pub flip_y: Option<bool>,
        // polygon
        pub color: Option<String>,
        pub vertices: Option<Vec<[f32; 2]>>,
        pub triangles: Option<Vec<u16>>,
        // sprite-renderer
        pub draw_mode: Option<String>,
        pub size: Option<[f32; 2]>,
    }

    #[derive(Deserialize)]
    #[serde(untagged)]
    pub enum ScaleSpec {
        Uniform(f32),
        PerAxis([f32; 2]),
    }
    impl ScaleSpec {
        pub fn resolve(&self) -> [f32; 2] {
            match self {
                Self::Uniform(s) => [*s, *s],
                Self::PerAxis(arr) => *arr,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_csa_tree() {
        let m = parse(
            r#"{
              "version": 1,
              "trees": [{
                "name": "X",
                "output": "csa",
                "root": {
                  "children": [
                    { "graphic": { "type":"sprite", "sprite":"foo" } }
                  ]
                }
              }]
            }"#,
        )
        .unwrap();
        assert_eq!(m.trees.len(), 1);
        let t = &m.trees[0];
        assert_eq!(t.output, Output::Csa);
        assert_eq!(t.scale, 1.0);
        assert_eq!(t.root.children.len(), 1);
        match &t.root.children[0].graphic.as_ref().unwrap() {
            Graphic::Sprite { method, ui_scale, .. } => {
                assert_eq!(*method, SpriteMethod::Id);
                assert_eq!(*ui_scale, 100.0);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_polygon_color_6_hex() {
        let m = parse(
            r#"{ "version": 1, "trees": [{
              "name": "X", "output": "csa",
              "root": { "children": [{
                "graphic": { "type":"polygon", "color":"32264D", "vertices":[[0,0],[1,0],[1,1]] }
              }]}
            }]}"#,
        )
        .unwrap();
        match &m.trees[0].root.children[0].graphic.as_ref().unwrap() {
            Graphic::Polygon { polygon_sprite, .. } => assert_eq!(polygon_sprite, "Color_32264DFF"),
            _ => panic!(),
        }
    }

    #[test]
    fn parse_polygon_color_8_hex() {
        let m = parse(
            r#"{ "version": 1, "trees": [{
              "name": "X", "output": "csa",
              "root": { "children": [{
                "graphic": { "type":"polygon", "color":"DEADBEEF", "vertices":[[0,0],[1,0],[1,1]] }
              }]}
            }]}"#,
        )
        .unwrap();
        match &m.trees[0].root.children[0].graphic.as_ref().unwrap() {
            Graphic::Polygon { polygon_sprite, .. } => assert_eq!(polygon_sprite, "Color_DEADBEEF"),
            _ => panic!(),
        }
    }

    #[test]
    fn parse_sma_output() {
        let m = parse(
            r#"{ "version": 1, "trees": [{
              "name": "X",
              "output": { "type":"sma", "fileId":-1234, "outputPath":"!Output/X.asset", "usedInCanvas":false },
              "root": { "children": [{
                "graphic": { "type":"sprite-renderer", "sprite":"foo" }
              }]}
            }]}"#,
        )
        .unwrap();
        match &m.trees[0].output {
            Output::Sma { file_id, output_path, used_in_canvas, keep_vertices, .. } => {
                assert_eq!(*file_id, -1234);
                assert_eq!(output_path, "!Output/X.asset");
                assert!(!*used_in_canvas);
                assert!(*keep_vertices); // default
            }
            _ => panic!(),
        }
        match &m.trees[0].root.children[0].graphic.as_ref().unwrap() {
            Graphic::SpriteRenderer { sprite, draw_mode, size } => {
                assert_eq!(sprite, "foo");
                assert_eq!(*draw_mode, DrawMode::Simple);
                assert!(size.is_none());
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_scale_uniform_and_per_axis() {
        let m = parse(
            r#"{ "version": 1, "trees": [{
              "name": "X", "output": "csa",
              "root": { "children": [
                { "scale": 2.5, "graphic": {"type":"sprite","sprite":"a"} },
                { "scale": [-1, 1], "graphic": {"type":"sprite","sprite":"b"} }
              ]}
            }]}"#,
        )
        .unwrap();
        let kids = &m.trees[0].root.children;
        assert_eq!(kids[0].scale, [2.5, 2.5]);
        assert_eq!(kids[1].scale, [-1.0, 1.0]);
    }

    #[test]
    fn parse_nested_children() {
        let m = parse(
            r#"{ "version": 1, "trees": [{
              "name": "X", "output": "csa",
              "root": { "children": [{
                "name": "Body",
                "pivot": [0.5, 0.4515571],
                "sizeDelta": [154, 181],
                "graphic": {"type":"sprite","sprite":"body"},
                "children": [{
                  "name": "SP",
                  "pos": [-69.8, -36.7],
                  "graphic": {"type":"sprite","sprite":"sp"}
                }]
              }]}
            }]}"#,
        )
        .unwrap();
        let body = &m.trees[0].root.children[0];
        assert_eq!(body.name, "Body");
        assert_eq!(body.pivot, [0.5, 0.4515571]);
        assert_eq!(body.children.len(), 1);
        let sp = &body.children[0];
        assert_eq!(sp.pos, [-69.8, -36.7]);
    }

    #[test]
    fn parse_rejects_unknown_top_level_field() {
        let m = parse(r#"{ "version": 1, "trees": [], "extra": 0 }"#);
        assert!(matches!(m, Err(ManifestError::Json(_))));
    }

    #[test]
    fn parse_rejects_bad_color() {
        let m = parse(
            r#"{ "version":1, "trees":[{
              "name":"X", "output":"csa",
              "root": { "children": [{
                "graphic": {"type":"polygon","color":"NOPE","vertices":[[0,0],[1,0],[1,1]]}
              }]}
            }]}"#,
        );
        assert!(matches!(m, Err(ManifestError::BadColor { .. })));
    }

    #[test]
    fn parse_rejects_unknown_method() {
        let m = parse(
            r#"{ "version":1, "trees":[{
              "name":"X", "output":"csa",
              "root": { "children": [{
                "graphic": {"type":"sprite","sprite":"a","method":"BOGUS"}
              }]}
            }]}"#,
        );
        assert!(matches!(m, Err(ManifestError::UnknownMethod { .. })));
    }

    #[test]
    fn parse_rejects_sma_as_tag() {
        let m = parse(
            r#"{ "version":1, "trees":[{
              "name":"X", "output":"sma",
              "root": { "children": [{ "graphic": {"type":"sprite-renderer","sprite":"a"} }]}
            }]}"#,
        );
        assert!(matches!(m, Err(ManifestError::OutputShape { .. })));
    }

    #[test]
    fn parse_rejects_duplicate_tree_name() {
        let m = parse(
            r#"{ "version":1, "trees":[
              {"name":"X","output":"csa","root":{"children":[{"graphic":{"type":"sprite","sprite":"a"}}]}},
              {"name":"X","output":"csa","root":{"children":[{"graphic":{"type":"sprite","sprite":"b"}}]}}
            ]}"#,
        );
        assert!(matches!(m, Err(ManifestError::DuplicateName(n)) if n == "X"));
    }
}
